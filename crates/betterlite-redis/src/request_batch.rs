pub mod batches;
pub mod prefetch;
pub mod registry;
pub mod scope;

mod tests;

use std::sync::{Mutex, MutexGuard};

pub type BackendKey = (String, Option<String>);

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
