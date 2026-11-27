use std::ops::Deref;
use std::sync::Arc;

use tokio::sync::Mutex;

mod tests;

// One handle is built at startup and cloned into each holder, since a static is banned
#[derive(Clone, Debug, Default)]
pub struct ModelReconcileLock(Arc<Mutex<()>>);

// Deref, not a lock() method, so clear_cache keeps taking a &Mutex<()> gate
impl Deref for ModelReconcileLock {
    type Target = Mutex<()>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
