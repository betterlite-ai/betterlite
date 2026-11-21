use std::sync::{Arc, PoisonError};

use tokio::sync::watch;
use tokio::time::{sleep, timeout};

use super::DiscoverySlots;
use super::clock::Clock;
use super::discover::discover_server;
use super::slot::SlotOutcome;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::oauth_resource::resource_identity::redact_mcp_resource_url;
use crate::registry::servers::ServerRegistry;
use crate::server_oauth::merge::merge_discovered;
use crate::server_oauth::unresolved::endpoints_unresolved;
use crate::server_record::MCPServer;

enum Attempt {
    Settled(SlotOutcome),
    Retryable,
    Final,
}

pub(super) async fn run<F, C>(
    slots: Arc<DiscoverySlots<F, C>>,
    server: MCPServer,
    generation: u64,
    sender: watch::Sender<Option<SlotOutcome>>,
) where
    F: MetadataFetcher + 'static,
    C: Clock + 'static,
{
    let limit = slots.settings.fetch.metadata_timeout;
    let outcome = match timeout(limit, resolve(&slots, &server, generation)).await {
        Ok(outcome) => outcome,
        Err(_elapsed) => SlotOutcome::TimedOut,
    };
    let server_id = &server.fields.server_id;
    let origin = redact_mcp_resource_url(server.fields.url.as_deref())
        .unwrap_or_else(|| "<no url>".to_owned());
    match &outcome {
        SlotOutcome::Resolved(_) => tracing::info!(
            "MCP OAuth discovery resolved the OAuth endpoints of server {server_id} from {origin}"
        ),
        SlotOutcome::TimedOut => {
            tracing::warn!(
                "Deferred MCP OAuth discovery timed out after {limit:?} for server {server_id} ({origin})"
            );
            record_failure(&slots, server_id, generation);
        }
        SlotOutcome::Incomplete => {
            tracing::warn!(
                "Deferred MCP OAuth discovery for server {server_id} ({origin}) returned incomplete metadata; the next request retries after the backoff"
            );
            record_failure(&slots, server_id, generation);
        }
        SlotOutcome::Stale => {}
    }
    sender.send_modify(|value| *value = Some(outcome));
}

async fn resolve<F, C>(
    slots: &DiscoverySlots<F, C>,
    server: &MCPServer,
    generation: u64,
) -> SlotOutcome
where
    F: MetadataFetcher + 'static,
    C: Clock + 'static,
{
    let mut delays = slots.settings.retry_delays.iter();
    loop {
        match attempt_once(slots, server, generation).await {
            Attempt::Settled(outcome) => return outcome,
            Attempt::Final => return SlotOutcome::Incomplete,
            Attempt::Retryable => {}
        }
        let Some(delay) = delays.next() else {
            return SlotOutcome::Incomplete;
        };
        sleep(*delay).await;
    }
}

async fn attempt_once<F, C>(
    slots: &DiscoverySlots<F, C>,
    server: &MCPServer,
    generation: u64,
) -> Attempt
where
    F: MetadataFetcher + 'static,
    C: Clock + 'static,
{
    let current = slots.registered_server(server).await;
    if !endpoints_unresolved(&current) {
        return Attempt::Settled(publish(slots, current, generation).await);
    }
    let attempt = discover_server(&slots.fetcher, slots.settings.fetch, &current).await;
    let candidate = merge_discovered(
        &slots.registered_server(server).await,
        attempt.metadata.as_ref(),
    );
    if endpoints_unresolved(&candidate) {
        return if attempt.retryable {
            Attempt::Retryable
        } else {
            Attempt::Final
        };
    }
    Attempt::Settled(publish(slots, candidate, generation).await)
}

async fn publish<F, C>(
    slots: &DiscoverySlots<F, C>,
    server: MCPServer,
    generation: u64,
) -> SlotOutcome
where
    F: MetadataFetcher + 'static,
    C: Clock + 'static,
{
    let now = slots.clock.now();
    let mut registry = slots.registry.write().await;
    let mut table = slots.table.lock().unwrap_or_else(PoisonError::into_inner);
    let server_id = &server.fields.server_id;
    if table
        .slots
        .get(server_id)
        .is_none_or(|slot| slot.generation != generation)
    {
        return SlotOutcome::Stale;
    }
    let ServerRegistry {
        db_servers,
        config_servers,
        ..
    } = &mut *registry;
    let registered = db_servers
        .get_mut(server_id)
        .or_else(|| config_servers.get_mut(server_id));
    if let Some(entry) = registered {
        entry.clone_from(&server);
        table.slots.remove(server_id);
    }
    drop(registry);
    if let Some(slot) = table.slots.get_mut(server_id) {
        slot.expires = Some(now.checked_add(slots.settings.temporary_ttl).unwrap_or(now));
    }
    drop(table);
    SlotOutcome::Resolved(Box::new(server))
}

fn record_failure<F, C>(slots: &DiscoverySlots<F, C>, server_id: &str, generation: u64)
where
    F: MetadataFetcher + 'static,
    C: Clock + 'static,
{
    let now = slots.clock.now();
    let mut table = slots.table.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(slot) = table
        .slots
        .get_mut(server_id)
        .filter(|held| held.generation == generation)
    else {
        return;
    };
    slot.consecutive_failures = slot.consecutive_failures.saturating_add(1);
    slot.retry_after = now
        .checked_add(slots.settings.backoff(slot.consecutive_failures))
        .unwrap_or(now);
    drop(table);
}
