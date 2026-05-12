//! Small utilities used across the crate.

use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Poison-safe `Mutex::lock`. Returns the guard even if the mutex is poisoned —
/// we don't keep invariants across panic boundaries, so taking the inner value
/// is strictly better than aborting the whole app.
pub trait MutexExt<T> {
    fn lock_or_poisoned(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    #[inline]
    fn lock_or_poisoned(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Poison-safe `RwLock` access. Same rationale as [`MutexExt`].
pub trait RwLockExt<T> {
    fn read_or_poisoned(&self) -> RwLockReadGuard<'_, T>;
    fn write_or_poisoned(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T> RwLockExt<T> for RwLock<T> {
    #[inline]
    fn read_or_poisoned(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(|e| e.into_inner())
    }
    #[inline]
    fn write_or_poisoned(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(|e| e.into_inner())
    }
}
