pub mod clock;
pub mod discover;
pub mod error;
mod join;
mod resolution;
pub mod settings;
pub mod slot;
mod tests;

use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;
use tokio_util::task::TaskTracker;

use self::clock::Clock;
use self::error::DiscoveryUnavailable;
use self::join::join;
use self::resolution::run;
use self::settings::DiscoverySettings;
use self::slot::{DiscoveryRun, DiscoverySlot, OutcomeWatch, SlotOutcome, SlotTable};
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::registry::servers::SharedRegistry;
use crate::server_oauth::unresolved::{endpoints_unresolved, requires_discovery};
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
            let table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
            let slot = table.slots.get(&fields.server_id);
            let has_slot = slot.is_some();
            let moved = slot
                .and_then(|held| held.run.as_ref())
                .is_some_and(|run| run.url != fields.url || run.auth_type != fields.auth_type);
            drop(table);
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

    #[must_use]
    pub fn prime_one(self: &Arc<Self>, server: &MCPServer) -> Option<OutcomeWatch> {
        let now = self.clock.now();
        let mut table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        table.sweep(now);
        let slot = table.slots.get_mut(&server.fields.server_id)?;
        if let Some(held) = &slot.run {
            // the run drops its sender when it returns, so a closed channel means it is done
            let finished = held.outcome.has_changed().is_err();
            let resolved = held
                .outcome
                .borrow()
                .as_ref()
                .is_some_and(|settled| matches!(settled, SlotOutcome::Resolved(_)));
            if !finished || now < slot.retry_after || resolved {
                return Some(held.outcome.clone());
            }
        }
        let (sender, outcome) = watch::channel(None);
        let task = run(Arc::clone(self), server.clone(), slot.generation, sender);
        slot.run = Some(DiscoveryRun {
            outcome: outcome.clone(),
            abort: self.tasks.spawn(task).abort_handle(),
            url: server.fields.url.clone(),
            auth_type: server.fields.auth_type,
        });
        drop(table);
        Some(outcome)
    }

    pub async fn ensure_discovered(
        self: &Arc<Self>,
        server: &MCPServer,
    ) -> Result<MCPServer, DiscoveryUnavailable> {
        if let Some(settled) = join(self, server).await {
            return settled;
        }
        if let Some(settled) = join(self, server).await {
            return settled;
        }
        let current = self.registered_server(server).await;
        if current.is_client_forwarded_token() || !endpoints_unresolved(&current) {
            return Ok(current);
        }
        Err(DiscoveryUnavailable::ChangedRepeatedly)
    }
}
