//! Background tick scheduler — the autonomic loop behind self-organization:
//! clusters thoughts, spawns/evicts child kerns, runs GNN propagation, decays
//! heat (`pulse`), and drives stigmergy cold-GC (spill-before-drop).

pub mod cluster;
pub mod gnn_propagate;
pub mod pulse;
pub mod queue;
pub mod stigmergy;
pub mod tasks;

use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;

use crate::base::constants::{KERN_COHESION_THRESHOLD, KERN_MIN_CLUSTER_SIZE};
use crate::base::graph::GraphGnn;
use crate::base::locks::{read_recovered, write_recovered};
use crate::config::TickConfig;
use crate::gnn::propagate::GnnConfig;

use cluster::{cohesion, is_core_cluster, vector_cluster, Cluster};
use gnn_propagate::do_gnn_propagate;
use queue::{task, task_extra, Queue, Task, TaskKind};
use tasks::{
	do_classify_contradiction, do_commit_access, do_disk_consolidate, do_enrich, do_name,
	do_persist, do_reembed, do_resolve, do_seed_questions, BroadcastQuestionFunc, EmbedFunc, LlmFunc,
};

/// Long-lived dependencies the tick worker carries across every task it
/// processes: the LLM / embed / broadcast hooks plus the GNN, tick, and
/// cold-tier config. Bundled so `start` and `process_task` take one context
/// instead of eight positional args (and drop their `too_many_arguments` allows).
/// All hooks are cheap `Arc` clones; the configs are small value types.
pub struct TickContext {
	pub llm: Option<LlmFunc>,
	pub embed: Option<EmbedFunc>,
	pub broadcast_q: Option<BroadcastQuestionFunc>,
	pub gnn_cfg: GnnConfig,
	pub tick_cfg: TickConfig,
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
		TaskKind::SeedQuestions => do_seed_questions(g, &t.extra, llm),
		TaskKind::ClassifyContradiction => do_classify_contradiction(q, g, &t.kern_id, &t.extra, llm, embed),
		TaskKind::Name => do_name(q, g, &t.kern_id, &ctx.tick_cfg, llm, embed),
		TaskKind::Enrich => do_enrich(q, g, &t.kern_id, &t.extra, llm, embed),
		TaskKind::ResolveQuestion => do_resolve(q, g, &t.kern_id, &t.extra, ctx.broadcast_q.as_ref()),
		TaskKind::Persist => do_persist(g, &t.kern_id),
		TaskKind::GnnPropagate => do_gnn_propagate(q, g, &t.kern_id, &ctx.gnn_cfg),
		TaskKind::StigmergyGc => stigmergy::run_gc(g, &t.kern_id),
		TaskKind::Reembed => do_reembed(g, &t.kern_id, embed),
		TaskKind::DiskConsolidate => do_disk_consolidate(g),
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
	let mut graph = write_recovered(g);

	// Phase 1: cluster the thoughts and decide which clusters spin out.
	let (clusters, spawn_indices) = match graph.kerns.get(kern_id) {
		Some(kern) => select_spawn_clusters(kern, tick_cfg.max_cluster_sample),
		None => return,
	};

	// Phase 2: spawn child kerns and migrate the selected clusters into them.
	let spawned_children = spawn_child_clusters(&mut graph, kern_id, &clusters, &spawn_indices);

	// Phase 3: gather follow-up work (enrich edges, resolve open questions).
	let (enrich_jobs, question_jobs) = match graph.kerns.get(kern_id) {
		Some(kern) => collect_follow_up_jobs(kern),
		None => {
			drop(graph);
			return;
		}
	};

	// Phase 4: reap empty unnamed children, reparenting any strays.
	let evicted = evict_empty_children(&mut graph, kern_id);

	let is_unnamed = graph
		.kerns
		.get(kern_id)
		.map(|k| k.is_unnamed())
		.unwrap_or(false);

	drop(graph);

	// Phase 5: enqueue the follow-up tasks discovered above (lock released).
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
		q.enqueue(task(TaskKind::Persist, kern_id));
	}
	// Skip GNN propagation when the cluster pass produced no structural change — the previous gnn_vector state is still valid.
	if did_structural_work {
		q.enqueue(task(TaskKind::GnnPropagate, kern_id));
	}
}

/// Phase 1 — cluster the kern's thoughts and pick which clusters are dense and
/// off-core enough to spin out into their own child kern. Pure read over `kern`;
/// returns every cluster plus the indices selected for spawning.
fn select_spawn_clusters(
	kern: &crate::base::types::Kern,
	max_sample: usize,
) -> (Vec<Cluster>, Vec<usize>) {
	// UNNAMED KERNS NEVER SPAWN. A fresh child is by construction one cohesive
	// cluster, and `do_cluster` spawns grandchildren (phase 2) before the Name
	// task is even enqueued (phase 5) — so an unnamed kern that may spawn
	// descends one level per pass, unboundedly (observed live: tens of
	// thousands of empty unnamed kerns). An unnamed kern also has an empty
	// `anchor_vec`, so the `is_core_cluster` skip below cannot protect it.
	// Holding its thoughts until Name succeeds is loss-free: nothing is moved,
	// dropped, or evicted, and the named kern splits off-core clusters normally.
	if !kern.is_named() {
		return (Vec::new(), Vec::new());
	}

	// `vector_cluster` requires `&[&Entity]`; materialize a Vec of refs because
	// `kern.entities` is a HashMap and produces an iterator, not a slice.
	let entities: Vec<_> = kern.entities.values().collect();
	let clusters = vector_cluster(&entities, max_sample);

	let mut spawn_indices = Vec::new();
	for (i, c) in clusters.iter().enumerate() {
		if is_core_cluster(c, &kern.anchor_vec) {
			continue;
		}
		if c.members.len() >= KERN_MIN_CLUSTER_SIZE && cohesion(&c.members) >= KERN_COHESION_THRESHOLD {
			spawn_indices.push(i);
		}
	}
	(clusters, spawn_indices)
}

/// Phase 2 — spawn one unnamed child kern per selected cluster and move that
/// cluster's thoughts out of the parent into it. Returns the new child ids.
fn spawn_child_clusters(
	graph: &mut GraphGnn,
	kern_id: &str,
	clusters: &[Cluster],
	spawn_indices: &[usize],
) -> Vec<String> {
	let mut spawned_children = Vec::new();
	for i in spawn_indices {
		// One DISTINCT child per cluster. Using `get_or_spawn_unnamed_child` here was
		// a bug: it reuses the parent's first unnamed child, so every selected cluster
		// collapsed into the SAME kern (and `spawned_children` carried that id N times,
		// enqueuing N duplicate Cluster/Persist tasks for it). `spawn_unnamed_child`
		// always creates a fresh child, so each cohesive cluster materialises as its
		// own sub-kern — the documented intent and what Phase 5's per-child task
		// enqueue assumes.
		let child_id = crate::base::accept::spawn_unnamed_child(graph, kern_id);
		for m in &clusters[*i].members {
			let entity_id = m.id.clone();
			let thought = graph
				.kerns
				.get_mut(kern_id)
				.and_then(|k| k.entities.remove(&entity_id));
			if let Some(t) = thought {
				if let Some(child) = graph.kerns.get_mut(&child_id) {
					child.entities.insert(entity_id, t);
				}
			}
		}
		spawned_children.push(child_id);
	}
	spawned_children
}

/// Phase 3 — collect follow-up work from the kern's edges: un-enriched real
/// edges (both endpoints still present) need an Enrich pass; dangling Question
/// edges (`to` empty) need resolution. Pure read; returns `(enrich, question)`.
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

/// Phase 4 — reap child kerns that are empty AND unnamed: reparent any stray
/// thoughts back to `kern_id`, deregister the child, and prune it from the
/// parent's child list. Returns whether any child was evicted.
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
	let graph = read_recovered(g);
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
			child.anchor_text = "named".into();
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
		// Real edge a->b (both endpoints present) -> enrich; dangling question -> resolve.
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
		// b is absent, so a->b is not enrichable.
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
		// Regression: spawn_child_clusters used get_or_spawn_unnamed_child, which
		// REUSES the parent's first unnamed child — so two selected clusters
		// collapsed into ONE kern (and `spawned_children` carried that id twice,
		// enqueuing duplicate Cluster/Persist tasks). Each cluster must materialise
		// as its OWN child, with no cross-contamination of members.
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
		// Regression: the unnamed-split runaway. A freshly spawned child is by
		// construction one cohesive cluster, and `do_cluster` spawns grandchildren
		// (phase 2) BEFORE its Name task runs (phase 5) — so an unnamed kern that
		// may spawn descends one level per tick pass forever (observed live:
		// tens of thousands of empty unnamed kerns). An unnamed kern has no
		// anchor_vec, so the is_core_cluster skip cannot protect it; it must not
		// spawn at all until named.
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
		// Counterpart guard: the unnamed gate must not disable legitimate splits.
		// A NAMED kern with a cohesive cluster orthogonal to its anchor still
		// spins that cluster out.
		let mut kern = Kern::new("k", "");
		kern.anchor_text = "named".into();
		kern.anchor_vec = vec![1.0, 0.0];
		assert!(kern.is_named(), "precondition: kern is named");
		for i in 0..crate::base::constants::KERN_MIN_CLUSTER_SIZE {
			let id = format!("e{i}");
			// Orthogonal to the anchor -> off-core, cohesive among themselves.
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
	fn do_cluster_skips_gnn_when_no_structural_work() {
		// Root kern with a single thought: below KERN_MIN_CLUSTER_SIZE so nothing
		// spawns, no edges so no enrich/question, no children so no eviction —
		// did_structural_work is false, so no GnnPropagate should be enqueued.
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
