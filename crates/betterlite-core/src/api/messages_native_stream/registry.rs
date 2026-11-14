use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::Notify;

// One process-wide registry of live pumps and detached drains, shared by every stream
#[derive(Default)]
pub struct StreamRegistry {
    live_pumps: Mutex<BTreeSet<u64>>,
    detached_drains: Mutex<BTreeSet<u64>>,
    next_id: AtomicU64,
    idle: Notify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistrySizes {
    pub live_pumps: usize,
    pub detached_drains: usize,
}

impl StreamRegistry {
    #[must_use]
    pub fn start_pump(self: &Arc<Self>) -> PumpRegistration {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.live_pumps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id);
        PumpRegistration {
            registry: Arc::clone(self),
            id,
        }
    }

    #[must_use]
    pub fn sizes(&self) -> RegistrySizes {
        let live_pumps = self
            .live_pumps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let detached_drains = self
            .detached_drains
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        RegistrySizes {
            live_pumps,
            detached_drains,
        }
    }

    // Shutdown awaits this: every pump and drain registered before the call has finished
    pub async fn wait_until_idle(&self) {
        loop {
            let notified = self.idle.notified();
            let sizes = self.sizes();
            if sizes.live_pumps == 0 && sizes.detached_drains == 0 {
                return;
            }
            notified.await;
        }
    }

    pub(super) fn claim_drain(self: &Arc<Self>, cap: usize) -> Option<DrainRegistration> {
        let mut drains = self
            .detached_drains
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if drains.len() >= cap {
            return None;
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        drains.insert(id);
        Some(DrainRegistration {
            registry: Arc::clone(self),
            id,
        })
    }
}

pub struct PumpRegistration {
    registry: Arc<StreamRegistry>,
    id: u64,
}

impl Drop for PumpRegistration {
    fn drop(&mut self) {
        self.registry
            .live_pumps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
        self.registry.idle.notify_waiters();
    }
}

pub(super) struct DrainRegistration {
    registry: Arc<StreamRegistry>,
    id: u64,
}

impl Drop for DrainRegistration {
    fn drop(&mut self) {
        self.registry
            .detached_drains
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
        self.registry.idle.notify_waiters();
    }
}
