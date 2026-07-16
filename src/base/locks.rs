//! Thin graph-lock acquisition wrappers.
//!
//! The graph and its sibling runtime locks are `parking_lot::{RwLock, Mutex}`.
//! parking_lot locks have **no poisoning** — a thread that panics while holding a
//! guard leaves the lock immediately usable by the next acquirer, so there is no
//! `PoisonError` to recover and no `unwrap()` that could turn a worker panic into a
//! daemon crash. These wrappers keep the historical `*_recovered` names (the many
//! call sites that predate the parking_lot swap) as a single acquisition point;
//! they now simply forward to `read` / `write` / `lock`.

use parking_lot::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Acquire a read guard.
pub fn read_recovered<T: ?Sized>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
	lock.read()
}

/// Acquire a write guard.
pub fn write_recovered<T: ?Sized>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
	lock.write()
}

/// Acquire a `Mutex` guard.
pub fn lock_recovered<T: ?Sized>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
	lock.lock()
}
