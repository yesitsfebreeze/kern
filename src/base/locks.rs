//! Poison-tolerant `RwLock` helpers.
//!
//! `std::sync::RwLock::{read, write}` return `Err(PoisonError)` when a thread
//! panicked while holding the write guard. Most kern call sites historically
//! `unwrap()` that result, which converts a worker-thread panic into a daemon
//! crash. The helpers in this module instead recover the inner guard via
//! `PoisonError::into_inner()`, log a warning through `tracing`, and hand the
//! caller a usable guard.
//!
//! # When to use
//!
//! Reach for these helpers from any kern code path where a single panicked
//! writer should not bring down the whole daemon — e.g. background tick
//! workers, gossip handlers, retrieval, MCP tool handlers. Prefer the helpers
//! over `lock.read().unwrap()` / `lock.write().unwrap()`.
//!
//! # What poison means
//!
//! A `RwLock` is poisoned when a thread panics while holding the write guard.
//! The lock is still memory-safe to access — Rust's borrow checker and the
//! lock's invariants are intact — but the protected value may have been left
//! in a *logically* inconsistent intermediate state by the aborted operation.
//!
//! # Why recovery is safe (caveats)
//!
//! - The thread that poisoned the lock is gone; it cannot continue mutating.
//! - The data is fully initialised (no `MaybeUninit`/uninit memory exposed).
//! - The remaining state is whatever the panicked operation had committed up
//!   to the panic point. Treat it as **best-effort**: invariants that span
//!   multiple fields may be temporarily broken until the next successful
//!   write restores them.
//!
//! Callers that require strict transactional consistency should not use these
//! helpers; they should propagate the error or rebuild from a known-good
//! snapshot. For the kern graph we accept best-effort recovery: a stale or
//! mid-update `GraphGnn` is preferable to a dead daemon.

use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Acquire a read guard, recovering from poison.
///
/// On poison, the inner guard is extracted via `PoisonError::into_inner()` and
/// a `warn!` is emitted via `tracing`. See module docs for safety reasoning.
pub fn read_recovered<T: ?Sized>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
	match lock.read() {
		Ok(g) => g,
		Err(poisoned) => {
			tracing::warn!(
				target: "kern::locks",
				"RwLock poisoned on read; recovering inner guard (best-effort, state may be partially mutated)"
			);
			poisoned.into_inner()
		}
	}
}

/// Acquire a write guard, recovering from poison.
///
/// On poison, the inner guard is extracted via `PoisonError::into_inner()` and
/// a `warn!` is emitted via `tracing`. See module docs for safety reasoning.
pub fn write_recovered<T: ?Sized>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
	match lock.write() {
		Ok(g) => g,
		Err(poisoned) => {
			tracing::warn!(
				target: "kern::locks",
				"RwLock poisoned on write; recovering inner guard (best-effort, state may be partially mutated)"
			);
			poisoned.into_inner()
		}
	}
}

/// Acquire a `Mutex` guard, recovering from poison.
///
/// On poison, the inner guard is extracted via `PoisonError::into_inner()` and
/// a `warn!` is emitted via `tracing`. See module docs for safety reasoning.
pub fn lock_recovered<T: ?Sized>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
	match lock.lock() {
		Ok(g) => g,
		Err(poisoned) => {
			tracing::warn!(
				target: "kern::locks",
				"Mutex poisoned on lock; recovering inner guard (best-effort, state may be partially mutated)"
			);
			poisoned.into_inner()
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::Arc;
	use std::thread;

	/// Poison an `RwLock<i32>` by panicking inside a held write guard AFTER
	/// committing `value`, so the recovered guard should observe `value`.
	fn poison_rwlock(value: i32) -> Arc<RwLock<i32>> {
		let lock = Arc::new(RwLock::new(0));
		let l = lock.clone();
		let h = thread::spawn(move || {
			let mut g = l.write().unwrap();
			*g = value; // committed before the panic
			panic!("intentional poison");
		});
		assert!(h.join().is_err(), "spawned thread panicked");
		assert!(lock.is_poisoned(), "lock is poisoned after a panic-in-write");
		lock
	}

	#[test]
	fn read_recovered_sees_committed_value_through_poison() {
		let lock = poison_rwlock(42);
		assert_eq!(*read_recovered(&lock), 42, "recovered read returns the pre-panic write");
	}

	#[test]
	fn write_recovered_yields_a_usable_guard_through_poison() {
		let lock = poison_rwlock(42);
		{
			let mut g = write_recovered(&lock);
			assert_eq!(*g, 42);
			*g = 7; // the recovered guard is fully writable
		}
		assert_eq!(*read_recovered(&lock), 7, "subsequent write took effect");
	}

	#[test]
	fn lock_recovered_sees_committed_value_through_poison() {
		let lock = Arc::new(Mutex::new(0));
		let l = lock.clone();
		let h = thread::spawn(move || {
			let mut g = l.lock().unwrap();
			*g = 99;
			panic!("intentional poison");
		});
		assert!(h.join().is_err());
		assert!(lock.is_poisoned());
		assert_eq!(*lock_recovered(&lock), 99);
	}

	#[test]
	fn helpers_pass_through_healthy_locks_unchanged() {
		let rw = RwLock::new(5);
		assert_eq!(*read_recovered(&rw), 5);
		*write_recovered(&rw) += 1;
		assert_eq!(*read_recovered(&rw), 6);
		let mx = Mutex::new(1);
		*lock_recovered(&mx) += 10;
		assert_eq!(*lock_recovered(&mx), 11);
	}
}
