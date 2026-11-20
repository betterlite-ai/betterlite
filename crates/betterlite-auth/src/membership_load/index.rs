use std::sync::PoisonError;
use std::sync::{Arc, Mutex};

use tokio::task::Id;

use super::load::MembershipLoad;
use crate::auth_invalidation::registry::{InflightLoad, MembershipInflight};

// What the index holds under a reservation key: a running load, or an entry the source's isinstance skips.
pub enum SharedEntry {
    Load(InflightLoad<MembershipLoad>),
    Foreign,
}

// The source stores an asyncio.Task; only a task joins, and B734's registry holds loads only.
pub trait MembershipIndex: Send + Sync {
    fn entry(&self, key: &str) -> Option<SharedEntry>;
    fn install(&self, key: &str, load: InflightLoad<MembershipLoad>);
    fn remove_if(&self, key: &str, id: Id);
}

// B734's registry has no compare-and-remove, so install and completion share one lock.
pub struct SharedMembershipIndex {
    registry: Arc<MembershipInflight<MembershipLoad>>,
    guard: Mutex<()>,
}

impl SharedMembershipIndex {
    #[must_use]
    pub fn new(registry: Arc<MembershipInflight<MembershipLoad>>) -> Self {
        Self {
            registry,
            guard: Mutex::new(()),
        }
    }
}

impl MembershipIndex for SharedMembershipIndex {
    fn entry(&self, key: &str) -> Option<SharedEntry> {
        self.registry.get(key).map(SharedEntry::Load)
    }

    fn install(&self, key: &str, load: InflightLoad<MembershipLoad>) {
        let _held = self.guard.lock().unwrap_or_else(PoisonError::into_inner);
        self.registry.install(key, load);
    }

    fn remove_if(&self, key: &str, id: Id) {
        let _held = self.guard.lock().unwrap_or_else(PoisonError::into_inner);
        if self.registry.get(key).is_some_and(|load| load.id == id) {
            self.registry.pop(key);
        }
    }
}
