pub mod batches;
pub mod prefetch;
pub mod registry;
pub mod scope;

mod backend_key;
mod tests;

use std::sync::{Mutex, MutexGuard};

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
