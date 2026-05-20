pub mod cluster;
pub mod gnn_propagate;
pub mod pulse;
pub mod queue;
pub mod stigmergy;
pub mod tasks;

use std::sync::{Arc, RwLock};
use std::time::Instant;

use crate::base::constants::{KERN_COHESION_THRESHOLD, KERN_MIN_CLUSTER_SIZE};
use crate::base::graph::GraphGnn;
use crate::base::locks::{read_recovered, write_recovered};
use crate::config::TickConfig;
use crate::gnn::propagate::GnnConfig;

use cluster::{cohesion, is_core_cluster, vector_cluster};
use gnn_propagate::do_gnn_propagate;
use queue::{task, task_extra, Queue, Task, TaskKind};
use tasks::{do_enrich, do_name, do_persist, do_resolve, BroadcastQuestionFunc, EmbedFunc, LlmFunc};

pub fn start(
	q: Arc<Queue>,
	g: Arc<RwLock<GraphGnn>>,
	llm: Option<LlmFunc>,
	embed: Option<EmbedFunc>,
	broadcast_q: Option<BroadcastQuestionFunc>,
	gnn_cfg: GnnConfig,
	tick_cfg: TickConfig,
) -> tokio::task::JoinHandle<()> {
	let mut rx = q.take_receiver().expect("receiver already taken");
	tokio::spawn(async move {
		while let Some(t) = rx.recv().await {
			let started = Instant::now();
			q.dequeued(&t);
			process_task(
				&q,
				&g,
				&t,
				llm.as_ref(),
				embed.as_ref(),
				broadcast_q.as_ref(),
				&gnn_cfg,
				&tick_cfg,
			);
			q.record_task_latency(started.elapsed());
			q.done();
		}
	})
}

fn process_task(
	q: &Queue,
	g: &Arc<RwLock<GraphGnn>>,
	t: &Task,
	llm: Option<&LlmFunc>,
	embed: Option<&EmbedFunc>,
	bq: Option<&BroadcastQuestionFunc>,
	gnn_cfg: &GnnConfig,
	tick_cfg: &TickConfig,
) {
	match t.kind {
		TaskKind::Cluster => do_cluster(q, g, &t.kern_id, tick_cfg, llm, embed),
		TaskKind::Split => {}
		TaskKind::Name => do_name(q, g, &t.kern_id, tick_cfg, llm, embed),
		TaskKind::Enrich => do_enrich(q, g, &t.kern_id, &t.extra, llm, embed),
		TaskKind::ResolveQuestion => do_resolve(q, g, &t.kern_id, &t.extra, bq),
		TaskKind::Persist => do_persist(g, &t.kern_id),
		TaskKind::GnnPropagate => do_gnn_propagate(q, g, &t.kern_id, gnn_cfg),
		TaskKind::StigmergyGc => stigmergy::run_gc(g, &t.kern_id),
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
	let (clusters, spawn_indices) = {
		let kern = match graph.kerns.get(kern_id) {
			Some(k) => k,
			None => return,
		};

		// `vector_cluster` requires `&[&Entity]`; we must materialize a Vec of refs
		// because `kern.entities` is a HashMap and produces an iterator, not a slice.
		let entities: Vec<_> = kern.entities.values().collect();
		let clusters = vector_cluster(&entities, tick_cfg.max_cluster_sample);
		let is_named = kern.is_named();

		let mut spawn_indices = Vec::new();
		for (i, c) in clusters.iter().enumerate() {
			if is_named && is_core_cluster(c, &kern.purpose_vec) {
				continue;
			}
			if c.members.len() >= KERN_MIN_CLUSTER_SIZE
				&& cohesion(&c.members) >= KERN_COHESION_THRESHOLD
			{
				spawn_indices.push(i);
			}
		}
		(clusters, spawn_indices)
	};

	let mut spawned_children = Vec::new();
	for i in &spawn_indices {
		let child_id = crate::base::accept::get_or_spawn_unnamed_child(&mut graph, kern_id);
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

	let kern = match graph.kerns.get(kern_id) {
		Some(k) => k,
		None => {
			drop(graph);
			return;
		}
	};
	let mut enrich_jobs = Vec::new();
	for r in kern.reasons.values() {
		if r.is_enriched()
			|| r.kind == crate::base::types::ReasonKind::Spawn
			|| r.kind == crate::base::types::ReasonKind::Question
		{
			continue;
		}
		if !kern.entities.contains_key(&r.from) || !kern.entities.contains_key(&r.to) {
			continue;
		}
		enrich_jobs.push(r.id.clone());
	}

	let mut question_jobs = Vec::new();
	for r in kern.reasons.values() {
		if r.kind == crate::base::types::ReasonKind::Question && r.to.is_empty() {
			question_jobs.push(r.id.clone());
		}
	}

	// SAFETY-borrow: clone required — eviction loop needs `&mut graph`
	// (deregister + get_mut), which conflicts with borrowing `kern.children`.
	let children_ids = kern.children.clone();
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
	let did_structural_work = !spawned_children.is_empty()
		|| evicted
		|| !enrich_jobs.is_empty()
		|| !question_jobs.is_empty();
	if !spawned_children.is_empty() || evicted {
		q.enqueue(task(TaskKind::Persist, kern_id));
	}
	// Skip GNN propagation when the cluster pass produced no structural change — the previous gnn_vector state is still valid.
	if did_structural_work {
		q.enqueue(task(TaskKind::GnnPropagate, kern_id));
	}
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

	let gnn_cfg = GnnConfig::defaults();
	let tick_cfg = TickConfig::default();

	let mut rx = q.take_receiver().unwrap();
	while let Ok(t) = rx.try_recv() {
		q.dequeued(&t);
		process_task(&q, &Arc::clone(g), &t, llm, embed, bq, &gnn_cfg, &tick_cfg);
		q.done();
	}
}
