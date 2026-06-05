use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use crate::base::locks::lock_recovered;

use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskKind {
	Cluster,
	Split,
	Name,
	Enrich,
	ResolveQuestion,
	Persist,
	GnnPropagate,
	/// Stigmergic cold-path garbage collection: drop thoughts whose pheromone
	/// has fully evaporated (cold + stale + non-durable). Dispatched in
	/// `tick::process_task` to `tick::stigmergy::run_gc`.
	StigmergyGc,
}

#[derive(Debug, Clone)]
pub struct Task {
	pub kind: TaskKind,
	pub kern_id: String,
	pub extra: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TaskKey {
	kind: TaskKind,
	kern_id: String,
	extra: String,
}

fn key_of(t: &Task) -> TaskKey {
	TaskKey {
		kind: t.kind,
		kern_id: t.kern_id.clone(),
		extra: t.extra.clone(),
	}
}

pub struct Queue {
	tx: mpsc::Sender<Task>,
	rx: Mutex<Option<mpsc::Receiver<Task>>>,
	pending: Mutex<HashMap<TaskKey, bool>>,
	inflight: std::sync::atomic::AtomicUsize,

	task_count: Mutex<i64>,
	task_latency_total: Mutex<Duration>,
}

impl Queue {
	pub fn new(size: usize) -> Self {
		let (tx, rx) = mpsc::channel(size);
		Self {
			tx,
			rx: Mutex::new(Some(rx)),
			pending: Mutex::new(HashMap::new()),
			inflight: std::sync::atomic::AtomicUsize::new(0),
			task_count: Mutex::new(0),
			task_latency_total: Mutex::new(Duration::ZERO),
		}
	}

	pub fn take_receiver(&self) -> Option<mpsc::Receiver<Task>> {
		lock_recovered(&self.rx).take()
	}

	pub fn enqueue(&self, t: Task) -> bool {
		let k = key_of(&t);
		{
			let mut pending = lock_recovered(&self.pending);
			if *pending.get(&k).unwrap_or(&false) {
				return false;
			}
			pending.insert(k, true);
		}
		self
			.inflight
			.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		if self.tx.try_send(t).is_err() {
			self
				.inflight
				.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
			return false;
		}
		true
	}

	pub fn dequeued(&self, t: &Task) {
		let k = key_of(t);
		lock_recovered(&self.pending).remove(&k);
	}

	pub fn done(&self) {
		self
			.inflight
			.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
	}

	pub fn pending_count(&self) -> usize {
		lock_recovered(&self.pending).len()
	}

	pub fn record_task_latency(&self, d: Duration) {
		let mut count = lock_recovered(&self.task_count);
		let mut total = lock_recovered(&self.task_latency_total);
		*count += 1;
		*total += d;
	}

	pub fn metrics(&self) -> (i64, i64) {
		let count = *lock_recovered(&self.task_count);
		let total = *lock_recovered(&self.task_latency_total);
		let avg = if count > 0 {
			total.as_millis() as i64 / count
		} else {
			0
		};
		(count, avg)
	}
}

pub fn task(kind: TaskKind, kern_id: &str) -> Task {
	Task {
		kind,
		kern_id: kern_id.to_string(),
		extra: String::new(),
	}
}

pub fn task_extra(kind: TaskKind, kern_id: &str, extra: &str) -> Task {
	Task {
		kind,
		kern_id: kern_id.to_string(),
		extra: extra.to_string(),
	}
}
