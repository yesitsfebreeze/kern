use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::Duration;

use crate::base::locks::lock_recovered;

use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskKind {
	Cluster,
	Name,
	Enrich,
	ResolveQuestion,
	// extra = entity id
	SeedQuestions,
	// extra = reason id
	ClassifyContradiction,
	Persist,
	GnnPropagate,
	StigmergyGc,
	Reembed,
	DiskConsolidate,
	// extra = newline-joined entity ids; kern_id empty
	CommitAccess,
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
	stats: Mutex<(i64, Duration)>,
}

impl Queue {
	pub fn new(size: usize) -> Self {
		let (tx, rx) = mpsc::channel(size);
		Self {
			tx,
			rx: Mutex::new(Some(rx)),
			pending: Mutex::new(HashMap::new()),
			inflight: std::sync::atomic::AtomicUsize::new(0),
			stats: Mutex::new((0, Duration::ZERO)),
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
			pending.insert(k.clone(), true);
		}
		self
			.inflight
			.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		if self.tx.try_send(t).is_err() {
			self
				.inflight
				.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
			// Roll back the pending marker too — else a full-channel failure flags
			// this key forever and dedup blocks every future re-enqueue.
			lock_recovered(&self.pending).remove(&k);
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
		let mut s = lock_recovered(&self.stats);
		s.0 += 1;
		s.1 += d;
	}

	pub fn metrics(&self) -> (i64, i64) {
		let (count, total) = *lock_recovered(&self.stats);
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

// ids newline-joined in `extra`; entity ids never contain a newline, so it round-trips.
pub fn task_commit_access(ids: &[String]) -> Task {
	Task {
		kind: TaskKind::CommitAccess,
		kern_id: String::new(),
		extra: ids.join("\n"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn enqueue_dedups_an_already_pending_key() {
		let q = Queue::new(8);
		assert!(q.enqueue(task(TaskKind::Cluster, "k")));
		assert!(
			!q.enqueue(task(TaskKind::Cluster, "k")),
			"same key is deduped"
		);
		assert_eq!(q.pending_count(), 1);
	}

	#[test]
	fn dequeued_clears_pending_so_the_key_can_re_enqueue() {
		let q = Queue::new(8);
		let t = task(TaskKind::Persist, "k");
		assert!(q.enqueue(t.clone()));
		assert!(!q.enqueue(t.clone()), "still pending -> deduped");
		q.dequeued(&t);
		assert_eq!(q.pending_count(), 0);
		assert!(q.enqueue(t), "re-enqueue succeeds after dequeue");
	}

	#[test]
	fn full_channel_send_failure_rolls_back_pending() {
		let q = Queue::new(1);
		assert!(q.enqueue(task(TaskKind::Cluster, "a")));
		let b = task(TaskKind::Cluster, "b");
		assert!(!q.enqueue(b.clone()), "full channel -> enqueue fails");
		assert_eq!(
			q.pending_count(),
			1,
			"only 'a' remains pending; 'b' was rolled back"
		);
		let mut rx = q.take_receiver().unwrap();
		let _ = rx.try_recv();
		assert!(q.enqueue(b), "b re-enqueues once a slot frees");
	}

	#[test]
	fn record_task_latency_accumulates_count_and_average() {
		let q = Queue::new(8);
		q.record_task_latency(Duration::from_millis(10));
		q.record_task_latency(Duration::from_millis(30));
		let (count, avg_ms) = q.metrics();
		assert_eq!(count, 2);
		assert_eq!(avg_ms, 20, "average latency = (10 + 30) / 2 ms");
	}
}
