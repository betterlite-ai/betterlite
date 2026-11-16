use betterlite_models::decisions::Json;

use crate::identity::{AgentIdentityFailure, FailureCode};

use super::AgentIdentityStore;
use super::records::{BindingKey, MICROSOFT_ENTRA_PROVIDER};
use super::unbound::unbound_client;

mod tests;

// LiteLLM's private binding lookup: the binding table first, then the retired table for an unbound client
pub(super) async fn bound_agent_id(
    store: &AgentIdentityStore,
    tenant_id: &str,
    client_id: &str,
) -> Result<Option<String>, AgentIdentityFailure> {
    let cache_key = binding_cache_key(tenant_id, client_id)?;
    if let Some(cached) = cached_agent_id(store, &cache_key).await {
        return Ok(Some(cached));
    }
    let key = BindingKey {
        provider: MICROSOFT_ENTRA_PROVIDER,
        tenant_id,
        client_id,
    };
    let row = store
        .identities
        .find_unique(&key)
        .await
        .map_err(|_source| AgentIdentityFailure {
            code: FailureCode::PolicyUnavailable,
            message: String::from("Agent identity could not be loaded"),
        })?;
    let Some(found) = row else {
        unbound_client(store, tenant_id, client_id).await?;
        return Ok(None);
    };
    remember_agent_id(store, &cache_key, &found.agent_id).await;
    Ok(Some(found.agent_id))
}

async fn cached_agent_id(store: &AgentIdentityStore, cache_key: &str) -> Option<String> {
    let cache = store.cache.as_ref()?;
    let cached = cache.read(cache_key).await?;
    match cached {
        Json::Text(agent_id) => Some(agent_id),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

async fn remember_agent_id(store: &AgentIdentityStore, cache_key: &str, agent_id: &str) {
    let Some(cache) = store.cache.as_ref() else {
        return;
    };
    let ttl = cache.management_ttl();
    cache
        .write(cache_key, &Json::Text(agent_id.to_owned()), ttl)
        .await;
}

// Python's json.dumps writes the (tenant_id, client_id) tuple as an array with ", " between its strings
fn binding_cache_key(tenant_id: &str, client_id: &str) -> Result<String, AgentIdentityFailure> {
    let (Ok(tenant), Ok(client)) = (
        serde_json::to_string(tenant_id),
        serde_json::to_string(client_id),
    ) else {
        return Err(AgentIdentityFailure {
            code: FailureCode::PolicyUnavailable,
            message: String::from("Agent identity could not be loaded"),
        });
    };
    Ok(format!("agent_identity:[{tenant}, {client}]"))
}
