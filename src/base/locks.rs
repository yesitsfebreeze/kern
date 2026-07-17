//! Thin graph-lock acquisition wrappers. parking_lot does not poison, so these
//! forward directly; the `_recovered` names are historical.

use parking_lot::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub fn read_recovered<T: ?Sized>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
	lock.read()
}

pub fn write_recovered<T: ?Sized>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
	lock.write()
}

pub fn lock_recovered<T: ?Sized>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
	lock.lock()
}
