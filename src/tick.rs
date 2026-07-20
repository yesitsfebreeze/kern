pub mod cluster;
pub mod gnn_propagate;
pub mod idle;
pub mod pulse;
pub mod queue;
pub mod stigmergy;
pub mod tasks;

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use crate::base::constants::{KERN_COHESION_THRESHOLD, KERN_MIN_CLUSTER_SIZE};
use crate::base::graph::GraphGnn;
use crate::base::heat::HeatConfig;
use crate::config::TickConfig;
use crate::gnn::propagate::GnnConfig;

use cluster::{cohesion, is_core_cluster, vector_cluster, Cluster};
use gnn_propagate::do_gnn_propagate;
use queue::{task, task_extra, Queue, Task, TaskKind};
use tasks::{
	do_classify_contradiction, do_commit_access, do_disk_consolidate, do_enrich, do_name, do_persist,
	do_reembed, do_resolve, do_seed_questions, BroadcastQuestionFunc, EmbedFunc, LlmFunc,
};

pub struct TickContext {
	pub llm: Option<LlmFunc>,
	pub embed: Option<EmbedFunc>,
	pub broadcast_q: Option<BroadcastQuestionFunc>,
	pub gnn_cfg: GnnConfig,
	pub tick_cfg: TickConfig,
	pub heat_cfg: HeatConfig,
}

pub fn start(
	q: Arc<Queue>,
	g: Arc<RwLock<GraphGnn>>,
	ctx: TickContext,
) -> tokio::task::JoinHandle<()> {
	let mut rx = q.take_receiver().expect("receiver already taken");
	tokio::spawn(async move {
		while let Some(t) = rx.recv().await {
			let started = Instant::now();
			q.dequeued(&t);
			process_task(&q, &g, &t, &ctx);
			q.record_task_latency(started.elapsed());
			q.done();
		}
	})
}

fn process_task(q: &Queue, g: &Arc<RwLock<GraphGnn>>, t: &Task, ctx: &TickContext) {
	let (llm, embed) = (ctx.llm.as_ref(), ctx.embed.as_ref());
	match t.kind {
		TaskKind::Cluster => do_cluster(q, g, &t.kern_id, &ctx.tick_cfg, llm, embed),
		TaskKind::SeedQuestions => do_seed_questions(q, g, &t.extra, llm),
		TaskKind::ClassifyContradiction => {
			do_classify_contradiction(q, g, &t.kern_id, &t.extra, llm, embed)
		}
		TaskKind::Name => do_name(q, g, &t.kern_id, &ctx.tick_cfg, llm, embed),
		TaskKind::Enrich => do_enrich(q, g, &t.kern_id, &t.extra, llm, embed),
		TaskKind::ResolveQuestion => do_resolve(q, g, &t.kern_id, &t.extra, ctx.broadcast_q.as_ref()),
		TaskKind::Persist => do_persist(g, &t.kern_id),
		TaskKind::GnnPropagate => do_gnn_propagate(q, g, &t.kern_id, &ctx.gnn_cfg),
		TaskKind::StigmergyGc => stigmergy::run_gc(g, &t.kern_id, &ctx.heat_cfg),
		TaskKind::Reembed => do_reembed(g, &t.kern_id, embed),
		TaskKind::DiskConsolidate => do_disk_consolidate(g),
		TaskKind::IdleSweep => {
			idle::run_idle_sweep(g, Duration::from_secs(ctx.tick_cfg.kern_idle_timeout_secs));
		}
		TaskKind::CommitAccess => do_commit_access(g, &t.extra),
	}
}

fn do_cluster(
	q: &Queue,
	g: &Arc<RwLock<GraphGnn>>,
	kern_id: &str,
	tick_cfg: &TickConfig,
	llm: Option<&LlmFunc>,
	_embed: Option<&EmbedFunc>,
) {
	let mut graph = g.write();

	let (clusters, spawn_indices) = match graph.kerns.get(kern_id) {
		Some(kern) => select_spawn_clusters(kern, tick_cfg.max_cluster_sample),
		None => return,
	};

	let spawned_children = spawn_child_clusters(&mut graph, kern_id, &clusters, &spawn_indices);

	let (enrich_jobs, question_jobs) = match graph.kerns.get(kern_id) {
		Some(kern) => collect_follow_up_jobs(kern),
		None => {
			drop(graph);
			return;
		}
	};

	let evicted = evict_empty_children(&mut graph, kern_id);

	let is_unnamed = graph
		.kerns
		.get(kern_id)
		.map(|k| k.is_unnamed())
		.unwrap_or(false);

	drop(graph);

	if is_unnamed && llm.is_some() {
		q.enqueue(task(TaskKind::Name, kern_id));
	}
	for child_id in &spawned_children {
		q.enqueue(task(TaskKind::Cluster, child_id));
	}
	for rid in &enrich_jobs {
		q.enqueue(task_extra(TaskKind::Enrich, kern_id, rid));
	}
	for rid in &question_jobs {
		q.enqueue(task_extra(TaskKind::ResolveQuestion, kern_id, rid));
	}
	let did_structural_work =
		!spawned_children.is_empty() || evicted || !enrich_jobs.is_empty() || !question_jobs.is_empty();
	if !spawned_children.is_empty() || evicted {
		// Persist children BEFORE the parent: parent-first + crash erases the
		// migrated entities from disk; child-first merely duplicates them briefly.
		for child_id in &spawned_children {
			q.enqueue(task(TaskKind::Persist, child_id));
		}
		q.enqueue(task(TaskKind::Persist, kern_id));
	}
	// No structural change -> previous gnn_vector state still valid; skip GNN.
	if did_structural_work {
		q.enqueue(task(TaskKind::GnnPropagate, kern_id));
	}
}

fn select_spawn_clusters(
	kern: &crate::base::types::Kern,
	max_sample: usize,
) -> (Vec<Cluster>, Vec<usize>) {
	// UNNAMED KERNS NEVER SPAWN — else each pass descends one level unboundedly
	// (see select_spawn_clusters_never_spawns_from_an_unnamed_kern).
	if !kern.is_named() {
		return (Vec::new(), Vec::new());
	}

	let entities: Vec<_> = kern.entities.values().collect();
	let clusters = vector_cluster(&entities, max_sample);

	let mut spawn_indices = Vec::new();
	for (i, c) in clusters.iter().enumerate() {
		if is_core_cluster(c, &kern.graviton_vec) {
			continue;
		}
		if c.members.len() >= KERN_MIN_CLUSTER_SIZE && cohesion(&c.members) >= KERN_COHESION_THRESHOLD {
			spawn_indices.push(i);
		}
	}
	(clusters, spawn_indices)
}

fn spawn_child_clusters(
	graph: &mut GraphGnn,
	kern_id: &str,
	clusters: &[Cluster],
	spawn_indices: &[usize],
) -> Vec<String> {
	let mut spawned_children = Vec::new();
	for i in spawn_indices {
		// One DISTINCT child per cluster: never `get_or_spawn_unnamed_child` — it
		// reuses the first unnamed child, collapsing every cluster into one kern.
		let child_id = crate::base::accept::spawn_unnamed_child(graph, kern_id);
		for m in &clusters[*i].members {
			// Carries outgoing reasons and reindexes; a rejected move leaves the entity put.
			if let Err(e) = crate::base::reason::move_entity(graph, kern_id, &child_id, &m.id) {
				tracing::warn!(
					target: "kern.cluster",
					from = %kern_id,
					to = %child_id,
					entity = %m.id,
					error = %e,
					"cluster migration skipped"
				);
			}
		}
		spawned_children.push(child_id);
	}
	spawned_children
}

fn collect_follow_up_jobs(kern: &crate::base::types::Kern) -> (Vec<String>, Vec<String>) {
	use crate::base::types::ReasonKind;

	let mut enrich_jobs = Vec::new();
	for r in kern.reasons.values() {
		if r.is_enriched() || r.kind == ReasonKind::Spawn || r.kind == ReasonKind::Question {
			continue;
		}
		if !kern.entities.contains_key(&r.from) || !kern.entities.contains_key(&r.to) {
			continue;
		}
		enrich_jobs.push(r.id.clone());
	}

	let mut question_jobs = Vec::new();
	for r in kern.reasons.values() {
		if r.kind == ReasonKind::Question && r.to.is_empty() {
			question_jobs.push(r.id.clone());
		}
	}
	(enrich_jobs, question_jobs)
}

fn evict_empty_children(graph: &mut GraphGnn, kern_id: &str) -> bool {
	let children_ids = match graph.kerns.get(kern_id) {
		Some(k) => k.children.clone(),
		None => return false,
	};

	let mut alive = Vec::new();
	let mut evicted = false;
	for child_id in &children_ids {
		let (named, has_thoughts, exists) = match graph.kerns.get(child_id) {
			Some(c) => (c.is_named(), !c.entities.is_empty(), true),
			None => (false, false, false),
		};
		if !exists || (!named && !has_thoughts) {
			if exists {
				let stray_ids: Vec<String> = graph
					.kerns
					.get(child_id)
					.map(|c| c.entities.keys().cloned().collect())
					.unwrap_or_default();
				for tid in stray_ids {
					let t = graph
						.kerns
						.get_mut(child_id)
						.and_then(|c| c.entities.remove(&tid));
					if let Some(t) = t {
						if let Some(parent) = graph.kerns.get_mut(kern_id) {
							parent.entities.insert(tid, t);
						}
					}
				}
			}
			graph.deregister(child_id);
			evicted = true;
			continue;
		}
		alive.push(child_id.clone());
	}
	if let Some(kern) = graph.kerns.get_mut(kern_id) {
		kern.children = alive;
	}
	evicted
}

pub fn enqueue_all(q: &Queue, g: &Arc<RwLock<GraphGnn>>) {
	let graph = g.read();
	for kern in graph.all() {
		if !kern.entities.is_empty() {
			q.enqueue(task(TaskKind::Cluster, &kern.id));
		}
	}
}

pub fn tick_sync(
	g: &Arc<RwLock<GraphGnn>>,
	kern_id: &str,
	llm: Option<&LlmFunc>,
	embed: Option<&EmbedFunc>,
	bq: Option<&BroadcastQuestionFunc>,
) {
	let q = Queue::new(256);
	q.enqueue(task(TaskKind::Cluster, kern_id));

	let ctx = TickContext {
		llm: llm.cloned(),
		embed: embed.cloned(),
		broadcast_q: bq.cloned(),
		gnn_cfg: GnnConfig::defaults(),
		tick_cfg: TickConfig::default(),
		heat_cfg: HeatConfig::default(),
	};

	let gg = Arc::clone(g);
	let mut rx = q.take_receiver().unwrap();
	while let Ok(t) = rx.try_recv() {
		q.dequeued(&t);
		process_task(&q, &gg, &t, &ctx);
		q.done();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::reason::add_reason;
	use crate::base::types::{Entity, Kern, Reason, ReasonKind};

	fn parent_child(child_named: bool, child_has_thought: bool) -> (GraphGnn, String, String) {
		let mut g = GraphGnn::new();
		let (pid, cid) = ("p".to_string(), "c".to_string());
		let mut parent = Kern::new(&pid, "");
		parent.children = vec![cid.clone()];
		let mut child = Kern::new(&cid, &pid);
		if child_named {
			child.graviton_text = "named".into();
		}
		if child_has_thought {
			child.entities.insert(
				"e1".into(),
				Entity {
					id: "e1".into(),
					..Default::default()
				},
			);
		}
		g.kerns.insert(pid.clone(), parent);
		g.kerns.insert(cid.clone(), child);
		(g, pid, cid)
	}

	#[test]
	fn evict_reaps_empty_unnamed_child() {
		let (mut g, pid, cid) = parent_child(false, false);
		assert!(evict_empty_children(&mut g, &pid));
		assert!(
			!g.kerns.contains_key(&cid),
			"empty unnamed child deregistered"
		);
		assert!(
			g.kerns.get(&pid).unwrap().children.is_empty(),
			"child pruned from parent"
		);
	}

	#[test]
	fn evict_keeps_unnamed_child_that_has_thoughts() {
		let (mut g, pid, cid) = parent_child(false, true);
		assert!(!evict_empty_children(&mut g, &pid));
		assert!(g.kerns.contains_key(&cid));
		assert_eq!(g.kerns.get(&pid).unwrap().children, vec![cid]);
	}

	#[test]
	fn collect_jobs_splits_enrich_and_question_edges() {
		let mut k = Kern::new("k", "");
		k.entities.insert(
			"a".into(),
			Entity {
				id: "a".into(),
				..Default::default()
			},
		);
		k.entities.insert(
			"b".into(),
			Entity {
				id: "b".into(),
				..Default::default()
			},
		);
		add_reason(
			&mut k,
			Reason {
				from: "a".into(),
				to: "b".into(),
				id: "a->b".into(),
				kind: ReasonKind::Similarity,
				..Default::default()
			},
		);
		add_reason(
			&mut k,
			Reason {
				from: "a".into(),
				to: String::new(),
				id: "q1".into(),
				kind: ReasonKind::Question,
				..Default::default()
			},
		);

		let (enrich, questions) = collect_follow_up_jobs(&k);
		assert_eq!(
			enrich,
			vec!["a->b".to_string()],
			"only the un-enriched real edge"
		);
		assert_eq!(
			questions,
			vec!["q1".to_string()],
			"only the open question edge"
		);
	}

	#[test]
	fn collect_jobs_skips_edges_with_missing_endpoint() {
		let mut k = Kern::new("k", "");
		k.entities.insert(
			"a".into(),
			Entity {
				id: "a".into(),
				..Default::default()
			},
		);
		add_reason(
			&mut k,
			Reason {
				from: "a".into(),
				to: "b".into(),
				id: "a->b".into(),
				kind: ReasonKind::Similarity,
				..Default::default()
			},
		);
		let (enrich, questions) = collect_follow_up_jobs(&k);
		assert!(enrich.is_empty(), "edge with a missing endpoint is skipped");
		assert!(questions.is_empty());
	}

	#[test]
	fn spawn_child_migrates_cluster_members_into_new_child() {
		let mut g = GraphGnn::new();
		let pid = "p".to_string();
		let mut parent = Kern::new(&pid, "");
		parent.entities.insert(
			"a".into(),
			Entity {
				id: "a".into(),
				..Default::default()
			},
		);
		parent.entities.insert(
			"b".into(),
			Entity {
				id: "b".into(),
				..Default::default()
			},
		);
		g.kerns.insert(pid.clone(), parent);

		let clusters = vec![Cluster {
			members: vec![
				Entity {
					id: "a".into(),
					..Default::default()
				},
				Entity {
					id: "b".into(),
					..Default::default()
				},
			],
		}];
		let spawned = spawn_child_clusters(&mut g, &pid, &clusters, &[0]);

		assert_eq!(spawned.len(), 1, "one selected cluster spawns one child");
		let child_id = &spawned[0];
		assert!(
			g.kerns.get(&pid).unwrap().entities.is_empty(),
			"cluster members moved out of the parent",
		);
		let child = g.kerns.get(child_id).expect("spawned child exists");
		assert!(
			child.entities.contains_key("a") && child.entities.contains_key("b"),
			"both members landed in the new child kern",
		);
	}

	#[test]
	fn spawn_child_clusters_creates_a_distinct_child_per_cluster() {
		let mut g = GraphGnn::new();
		let pid = "p".to_string();
		let mut parent = Kern::new(&pid, "");
		for id in ["a", "b", "c", "d"] {
			parent.entities.insert(
				id.into(),
				Entity {
					id: id.into(),
					..Default::default()
				},
			);
		}
		g.kerns.insert(pid.clone(), parent);

		let clusters = vec![
			Cluster {
				members: vec![
					Entity {
						id: "a".into(),
						..Default::default()
					},
					Entity {
						id: "b".into(),
						..Default::default()
					},
				],
			},
			Cluster {
				members: vec![
					Entity {
						id: "c".into(),
						..Default::default()
					},
					Entity {
						id: "d".into(),
						..Default::default()
					},
				],
			},
		];
		let spawned = spawn_child_clusters(&mut g, &pid, &clusters, &[0, 1]);

		assert_eq!(spawned.len(), 2, "two selected clusters spawn two ids");
		assert_ne!(
			spawned[0], spawned[1],
			"each cluster lands in a DISTINCT child kern"
		);
		let c0 = g.kerns.get(&spawned[0]).expect("first child exists");
		assert!(
			c0.entities.contains_key("a") && c0.entities.contains_key("b"),
			"cluster 0 members"
		);
		assert!(
			!c0.entities.contains_key("c") && !c0.entities.contains_key("d"),
			"no cross-contamination"
		);
		let c1 = g.kerns.get(&spawned[1]).expect("second child exists");
		assert!(
			c1.entities.contains_key("c") && c1.entities.contains_key("d"),
			"cluster 1 members"
		);
		assert!(
			g.kerns.get(&pid).unwrap().entities.is_empty(),
			"all clustered members moved out of the parent",
		);
	}

	#[test]
	fn select_spawn_clusters_never_spawns_from_an_unnamed_kern() {
		let mut kern = Kern::new("k", "");
		assert!(!kern.is_named(), "precondition: kern is unnamed");
		for i in 0..crate::base::constants::KERN_MIN_CLUSTER_SIZE {
			let id = format!("e{i}");
			kern.entities.insert(
				id.clone(),
				Entity {
					id,
					vector: vec![1.0, 0.0],
					..Default::default()
				},
			);
		}

		let (_, spawn_indices) = select_spawn_clusters(&kern, 100);
		assert!(
			spawn_indices.is_empty(),
			"an unnamed kern must never spawn children; got {spawn_indices:?}",
		);
	}

	#[test]
	fn select_spawn_clusters_still_spawns_off_core_cluster_from_named_kern() {
		let mut kern = Kern::new("k", "");
		kern.graviton_text = "named".into();
		kern.graviton_vec = vec![1.0, 0.0];
		assert!(kern.is_named(), "precondition: kern is named");
		for i in 0..crate::base::constants::KERN_MIN_CLUSTER_SIZE {
			let id = format!("e{i}");
			kern.entities.insert(
				id.clone(),
				Entity {
					id,
					vector: vec![0.0, 1.0],
					..Default::default()
				},
			);
		}

		let (_, spawn_indices) = select_spawn_clusters(&kern, 100);
		assert_eq!(
			spawn_indices.len(),
			1,
			"a named kern's off-core cohesive cluster still spawns",
		);
	}

	#[test]
	fn do_cluster_persists_each_spawned_child_before_the_parent() {
		let q = Queue::new(64);
		let mut g = GraphGnn::new();
		let mut kern = Kern::new("k", "");
		kern.graviton_text = "named".into();
		kern.graviton_vec = vec![1.0, 0.0];
		for i in 0..crate::base::constants::KERN_MIN_CLUSTER_SIZE {
			let id = format!("e{i}");
			kern.entities.insert(
				id.clone(),
				Entity {
					id,
					vector: vec![0.0, 1.0],
					..Default::default()
				},
			);
		}
		g.kerns.insert("k".into(), kern);
		let g = Arc::new(RwLock::new(g));

		do_cluster(&q, &g, "k", &TickConfig::default(), None, None);

		let children = g.read().kerns.get("k").unwrap().children.clone();
		assert!(!children.is_empty(), "precondition: a child spawned");

		let mut rx = q.take_receiver().unwrap();
		let mut persists = Vec::new();
		while let Ok(t) = rx.try_recv() {
			if matches!(t.kind, TaskKind::Persist) {
				persists.push(t.kern_id.clone());
			}
		}
		let parent_pos = persists
			.iter()
			.position(|k| k == "k")
			.expect("parent Persist enqueued");
		for c in &children {
			let child_pos = persists
				.iter()
				.position(|k| k == c)
				.unwrap_or_else(|| panic!("spawned child {c} gets its own Persist"));
			assert!(
				child_pos < parent_pos,
				"child Persist must run before the parent row drops the migrated entities"
			);
		}
	}

	#[test]
	fn spawning_a_cluster_carries_outgoing_reasons_and_reindexes_the_entity() {
		use crate::base::reason::add_reason;
		use crate::base::types::{Kern, Reason};

		let mut g = GraphGnn::new();
		let root_id = g.root.id.clone();
		let mut parent = Kern::new("parent", &root_id);
		for id in ["moved", "stays"] {
			parent.entities.insert(
				id.into(),
				Entity {
					id: id.into(),
					..Default::default()
				},
			);
		}
		add_reason(
			&mut parent,
			Reason {
				id: "out".into(),
				from: "moved".into(),
				to: "stays".into(),
				..Default::default()
			},
		);
		g.register(parent);
		g.index_entity("moved", "parent");
		g.index_entity("stays", "parent");
		g.index_reason("out", "parent");

		let clusters = vec![Cluster {
			members: vec![Entity {
				id: "moved".into(),
				..Default::default()
			}],
		}];
		let children = spawn_child_clusters(&mut g, "parent", &clusters, &[0]);
		let child_id = children.first().expect("one child spawned").clone();

		assert_eq!(
			g.kern_of_entity("moved"),
			Some(child_id.as_str()),
			"entity->kern index must follow the entity into the child"
		);

		let child = g.kerns.get(&child_id).expect("child resident");
		assert!(child.entities.contains_key("moved"));
		assert!(
			child.reasons.contains_key("out"),
			"an edge lives in its `from` kern, so the outgoing reason moves too"
		);
		assert_eq!(
			g.kern_of_reason("out"),
			Some(child_id.as_str()),
			"reason->kern index must follow the reason"
		);

		let parent = g.kerns.get("parent").expect("parent resident");
		assert!(
			!parent.reasons.contains_key("out"),
			"the reason must not be left behind in the parent"
		);
		assert!(
			parent.entities.contains_key("stays"),
			"unclustered entities stay put"
		);
	}

	#[test]
	fn a_failed_cluster_migration_never_drops_the_entity() {
		let mut g = GraphGnn::new();
		let root_id = g.root.id.clone();
		let mut parent = crate::base::types::Kern::new("parent", &root_id);
		parent.entities.insert(
			"e1".into(),
			Entity {
				id: "e1".into(),
				..Default::default()
			},
		);
		g.register(parent);

		// A member that is not actually in the source kern: the move must be rejected, not lossy.
		let clusters = vec![Cluster {
			members: vec![Entity {
				id: "ghost".into(),
				..Default::default()
			}],
		}];
		spawn_child_clusters(&mut g, "parent", &clusters, &[0]);

		assert!(
			g.kerns
				.get("parent")
				.expect("parent resident")
				.entities
				.contains_key("e1"),
			"a rejected move leaves the parent intact"
		);
	}

	#[test]
	fn do_cluster_skips_gnn_when_no_structural_work() {
		let q = Queue::new(64);
		let mut g = GraphGnn::new();
		let root_id = g.root.id.clone();
		if let Some(k) = g.kerns.get_mut(&root_id) {
			k.entities.insert(
				"e1".into(),
				Entity {
					id: "e1".into(),
					..Default::default()
				},
			);
		}
		let g = Arc::new(RwLock::new(g));

		do_cluster(&q, &g, &root_id, &TickConfig::default(), None, None);

		let mut rx = q.take_receiver().unwrap();
		let mut kinds = Vec::new();
		while let Ok(t) = rx.try_recv() {
			kinds.push(t.kind);
		}
		let gnn = kinds
			.iter()
			.filter(|k| matches!(k, TaskKind::GnnPropagate))
			.count();
		assert_eq!(gnn, 0, "no structural change -> GNN propagation skipped");
	}
}
