use std::ops::Deref;
use std::sync::Arc;

use tokio::sync::Mutex;

mod tests;

// One handle is built at startup and cloned into each holder, since a static is banned
#[derive(Clone, Debug, Default)]
pub struct ModelReconcileLock(Arc<Mutex<()>>);

// ast-grep-ignore: no-deref-impls - clear_cache takes a &Mutex<()> gate, so holders deref to it
impl Deref for ModelReconcileLock {
    type Target = Mutex<()>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
