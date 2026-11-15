use betterlite_models::decisions::Json;

use crate::identity::{AgentIdentityFailure, FailureCode};

use super::AgentIdentityStore;
use super::records::MICROSOFT_ENTRA_PROVIDER;

mod tests;

impl AgentIdentityStore {
    pub(super) async fn bound_agent_id(
        &self,
        tenant_id: &str,
        client_id: &str,
    ) -> Result<Option<String>, AgentIdentityFailure> {
        let cache_key = binding_cache_key(tenant_id, client_id)?;
        if let Some(cached) = self.cached_agent_id(&cache_key).await {
            return Ok(Some(cached));
        }
        let row = self
            .identities
            .find_unique(MICROSOFT_ENTRA_PROVIDER, tenant_id, client_id)
            .await
            .map_err(|_source| identity_unavailable())?;
        let Some(row) = row else {
            self.unbound_client(tenant_id, client_id).await?;
            return Ok(None);
        };
        self.remember_agent_id(&cache_key, &row.agent_id).await;
        Ok(Some(row.agent_id))
    }

    async fn cached_agent_id(&self, cache_key: &str) -> Option<String> {
        let cache = self.cache.as_ref()?;
        let cached = cache.read(cache_key).await?;
        match cached {
            Json::Text(agent_id) => Some(agent_id),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        }
    }

    async fn remember_agent_id(&self, cache_key: &str, agent_id: &str) {
        let Some(cache) = self.cache.as_ref() else {
            return;
        };
        let ttl = cache.management_ttl();
        cache
            .write(cache_key, &Json::Text(agent_id.to_owned()), ttl)
            .await;
    }
}

// Python's json.dumps writes the (tenant_id, client_id) tuple as an array with ", " between its strings
fn binding_cache_key(tenant_id: &str, client_id: &str) -> Result<String, AgentIdentityFailure> {
    let Ok(tenant) = serde_json::to_string(tenant_id) else {
        return Err(identity_unavailable());
    };
    let Ok(client) = serde_json::to_string(client_id) else {
        return Err(identity_unavailable());
    };
    Ok(format!("agent_identity:[{tenant}, {client}]"))
}

fn identity_unavailable() -> AgentIdentityFailure {
    AgentIdentityFailure {
        code: FailureCode::PolicyUnavailable,
        message: String::from("Agent identity could not be loaded"),
    }
}
