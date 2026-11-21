use std::sync::Arc;

use super::DiscoverySlots;
use super::clock::Clock;
use super::error::DiscoveryUnavailable;
use super::slot::{FailureReason, SlotOutcome};
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::server_record::MCPServer;

pub(super) async fn join<F, C>(
    slots: &Arc<DiscoverySlots<F, C>>,
    server: &MCPServer,
) -> Option<Result<MCPServer, DiscoveryUnavailable>>
where
    F: MetadataFetcher + 'static,
    C: Clock + 'static,
{
    let Some(mut outcome) = slots.prime_one(server) else {
        return Some(Ok(slots.registered_server(server).await));
    };
    let finished = outcome
        .wait_for(Option::is_some)
        .await
        .ok()
        .and_then(|value| value.clone());
    let reason = match finished? {
        SlotOutcome::Resolved(resolved) => return Some(Ok(*resolved)),
        SlotOutcome::Stale => return None,
        SlotOutcome::TimedOut => FailureReason::TimedOut,
        SlotOutcome::Incomplete => FailureReason::Incomplete,
    };
    let current = slots.registered_server(server).await;
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
