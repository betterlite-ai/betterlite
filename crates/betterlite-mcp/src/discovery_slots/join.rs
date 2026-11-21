use std::sync::{Arc, PoisonError};

use tokio::sync::watch;

use super::DiscoverySlots;
use super::clock::Clock;
use super::error::DiscoveryUnavailable;
use super::slot::{DiscoveryRun, OutcomeWatch, SlotOutcome};
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::server_oauth::unresolved::endpoints_unresolved;
use crate::server_record::MCPServer;

impl<F, C> DiscoverySlots<F, C>
where
    F: MetadataFetcher + 'static,
    C: Clock + 'static,
{
    #[must_use]
    pub fn prime_one(self: &Arc<Self>, server: &MCPServer) -> Option<OutcomeWatch> {
        let now = self.clock.now();
        let mut table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        table.sweep(now);
        let slot = table.slots.get_mut(&server.fields.server_id)?;
        if let Some(run) = &slot.run {
            // the run drops its sender when it returns, so a closed channel means it is done
            let finished = run.outcome.has_changed().is_err();
            let resolved = matches!(*run.outcome.borrow(), Some(SlotOutcome::Resolved(_)));
            if !finished || now < slot.retry_after || resolved {
                return Some(run.outcome.clone());
            }
        }
        let (sender, outcome) = watch::channel(None);
        let task = Arc::clone(self).run(server.clone(), slot.generation, sender);
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
        if let Some(settled) = self.join(server).await {
            return settled;
        }
        if let Some(settled) = self.join(server).await {
            return settled;
        }
        let current = self.registered_server(server).await;
        if current.is_client_forwarded_token() || !endpoints_unresolved(&current) {
            return Ok(current);
        }
        Err(DiscoveryUnavailable::ChangedRepeatedly)
    }

    async fn join(
        self: &Arc<Self>,
        server: &MCPServer,
    ) -> Option<Result<MCPServer, DiscoveryUnavailable>> {
        let Some(mut outcome) = self.prime_one(server) else {
            return Some(Ok(self.registered_server(server).await));
        };
        let finished = match outcome.wait_for(Option::is_some).await {
            Ok(value) => value.clone(),
            Err(_closed) => None,
        };
        let reason = match finished? {
            SlotOutcome::Resolved(resolved) => return Some(Ok(*resolved)),
            SlotOutcome::Stale => return None,
            SlotOutcome::Failed(reason) => reason,
        };
        let current = self.registered_server(server).await;
        if current.is_client_forwarded_token() {
            return Some(Ok(current));
        }
        let fields = &current.fields;
        let server_ref = fields
            .alias
            .as_deref()
            .or(fields.server_name.as_deref())
            .unwrap_or(&fields.name);
        Some(Err(DiscoveryUnavailable::Failed {
            reason,
            server_ref: server_ref.to_owned(),
        }))
    }
}
