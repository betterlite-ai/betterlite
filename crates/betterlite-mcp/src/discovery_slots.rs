pub mod clock;
pub mod discover;
pub mod error;
mod join;
mod resolution;
pub mod settings;
pub mod slot;
mod tests;

use std::sync::{Arc, Mutex, PoisonError};

use tokio_util::task::TaskTracker;

use self::clock::Clock;
use self::settings::DiscoverySettings;
use self::slot::{DiscoverySlot, SlotTable};
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::registry::servers::SharedRegistry;
use crate::server_oauth::unresolved::requires_discovery;
use crate::server_record::MCPServer;

pub struct DiscoverySlots<F, C> {
    registry: SharedRegistry,
    fetcher: F,
    clock: C,
    settings: DiscoverySettings,
    tasks: TaskTracker,
    table: Mutex<SlotTable>,
}

impl<F, C> DiscoverySlots<F, C>
where
    F: MetadataFetcher + 'static,
    C: Clock + 'static,
{
    #[must_use]
    pub fn new(
        registry: SharedRegistry,
        fetcher: F,
        clock: C,
        settings: DiscoverySettings,
        tasks: TaskTracker,
    ) -> Self {
        Self {
            registry,
            fetcher,
            clock,
            settings,
            tasks,
            table: Mutex::new(SlotTable::default()),
        }
    }

    pub async fn registered_server(&self, server: &MCPServer) -> MCPServer {
        let registry = self.registry.read().await;
        let server_id = &server.fields.server_id;
        registry
            .db_servers
            .get(server_id)
            .or_else(|| registry.config_servers.get(server_id))
            .unwrap_or(server)
            .clone()
    }

    #[must_use]
    pub fn slot_exists(&self, server_id: &str) -> bool {
        let now = self.clock.now();
        let mut table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        table.sweep(now);
        table.slots.contains_key(server_id)
    }

    pub fn set_deferred(&self, server_id: &str, discovery_deferred: bool) {
        self.invalidate_state(server_id);
        if !discovery_deferred {
            return;
        }
        let now = self.clock.now();
        let mut table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        table.generation = table.generation.wrapping_add(1);
        let slot = DiscoverySlot {
            generation: table.generation,
            consecutive_failures: 0,
            retry_after: now,
            expires: None,
            run: None,
        };
        table.slots.insert(server_id.to_owned(), slot);
    }

    pub fn invalidate_state(&self, server_id: &str) {
        let mut table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        let removed = table.slots.remove(server_id).and_then(|slot| slot.run);
        drop(table);
        if let Some(run) = removed {
            run.abort.abort();
        }
    }

    pub fn reconcile(&self, servers: &[MCPServer]) {
        for server in servers {
            let fields = &server.fields;
            let should_defer =
                requires_discovery(fields.url.as_deref(), fields.issuer_is_anchored, server);
            let (has_slot, moved) = {
                let table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
                let slot = table.slots.get(&fields.server_id);
                let moved = slot
                    .and_then(|held| held.run.as_ref())
                    .is_some_and(|run| run.url != fields.url || run.auth_type != fields.auth_type);
                (slot.is_some(), moved)
            };
            if moved || should_defer != has_slot {
                self.set_deferred(&fields.server_id, should_defer);
            }
        }
    }

    pub fn prime(self: &Arc<Self>, servers: &[MCPServer]) {
        for server in servers {
            drop(self.prime_one(server));
        }
    }
}
