use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

// One process-wide registry of live pumps and detached drains, shared by every stream
#[derive(Default)]
pub struct StreamRegistry {
    pumps: Mutex<BTreeSet<usize>>,
    drains: Mutex<BTreeSet<usize>>,
    next_id: AtomicUsize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistrySizes {
    pub live_pumps: usize,
    pub detached_drains: usize,
}

impl StreamRegistry {
    pub(super) fn register_pump(self: &Arc<Self>) -> Slot {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let _inserted = self
            .pumps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id);
        Slot {
            registry: Arc::clone(self),
            id,
            kind: SlotKind::Pump,
        }
    }

    #[must_use]
    pub fn sizes(&self) -> RegistrySizes {
        let live_pumps = self
            .pumps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let detached_drains = self
            .drains
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        RegistrySizes {
            live_pumps,
            detached_drains,
        }
    }

    pub(super) fn claim_drain(self: &Arc<Self>, cap: usize) -> Option<Slot> {
        let mut drains = self.drains.lock().unwrap_or_else(PoisonError::into_inner);
        if drains.len() >= cap {
            return None;
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let _inserted = drains.insert(id);
        drop(drains);
        Some(Slot {
            registry: Arc::clone(self),
            id,
            kind: SlotKind::Drain,
        })
    }

    const fn set(&self, kind: SlotKind) -> &Mutex<BTreeSet<usize>> {
        match kind {
            SlotKind::Pump => &self.pumps,
            SlotKind::Drain => &self.drains,
        }
    }

    fn release(&self, kind: SlotKind, id: usize) {
        let _removed = self
            .set(kind)
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
    }
}

#[derive(Clone, Copy)]
enum SlotKind {
    Pump,
    Drain,
}

// Dropping either guard releases its entry when the task ends
pub struct Slot {
    registry: Arc<StreamRegistry>,
    id: usize,
    kind: SlotKind,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.registry.release(self.kind, self.id);
    }
}
