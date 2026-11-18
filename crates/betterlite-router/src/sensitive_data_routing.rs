use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

pub mod error;
pub mod identity;
pub mod ports;

mod tests;

use self::error::SensitiveRoutingError;
use self::identity::PrincipalIdentity;
use self::ports::SensitiveRoutingCache;

// _make_cache_key's "{sensitive_route:<tenant>:<session_id>}:model"; the read and the write build it alike
const CACHE_PREFIX: &str = "sensitive_route:";
const MODEL_FIELD: &str = "model";
const METADATA_FIELD: &str = "metadata";
const APPLIED_FIELD: &str = "sensitive_data_routing_applied";
const ORIGINAL_MODEL_FIELD: &str = "sensitive_data_routing_original_model";

pub struct SensitiveDataRoutingHandler<C: SensitiveRoutingCache> {
    pub internal_usage_cache: C,
    // __init__: LITELLM_SENSITIVE_ROUTING_TTL, default 3600, arrives through the config section
    pub ttl_seconds: u64,
}

impl<C: SensitiveRoutingCache> SensitiveDataRoutingHandler<C> {
    // async_pre_call_hook: an existing override replaces the requested model before the call
    pub async fn async_pre_call_hook(
        &self,
        data: &mut JsonMap,
        session_id: Option<&str>,
        principal: PrincipalIdentity<'_>,
    ) -> Result<(), SensitiveRoutingError> {
        let Some(session) = session_id else {
            return Ok(());
        };
        let Some(routed_model) = self.routed_model(session, principal).await else {
            return Ok(());
        };
        let original_model = data.get(MODEL_FIELD).cloned();
        if original_model.as_ref().and_then(JsonValue::as_str) == Some(routed_model.as_str()) {
            return Ok(());
        }
        tracing::info!(
            session_id = session,
            original_model = ?original_model,
            routed_model = %routed_model,
            "SensitiveDataRoutingHandler: Applying session routing override"
        );
        if data.get(METADATA_FIELD).is_none_or(JsonValue::is_null) {
            let _previous_metadata =
                data.insert(METADATA_FIELD.to_owned(), JsonValue::Object(JsonMap::new()));
        }
        let Some(JsonValue::Object(metadata)) = data.get_mut(METADATA_FIELD) else {
            return Err(SensitiveRoutingError::MetadataNotObject);
        };
        metadata.insert(APPLIED_FIELD.to_owned(), JsonValue::Bool(true));
        metadata.insert(
            ORIGINAL_MODEL_FIELD.to_owned(),
            original_model.unwrap_or(JsonValue::Null),
        );
        let _previous_model = data.insert(MODEL_FIELD.to_owned(), JsonValue::String(routed_model));
        Ok(())
    }

    // _get_routed_model: the Redis tier answers first, and the local tier covers its absence or failure
    async fn routed_model(
        &self,
        session_id: &str,
        principal: PrincipalIdentity<'_>,
    ) -> Option<String> {
        let tenant = resolve_tenant(principal);
        let cache_key = format!("{{{CACHE_PREFIX}{tenant}:{session_id}}}:model");
        if let Some(routed_model) = self.redis_routed_model(&cache_key).await {
            return Some(routed_model);
        }
        // async_get_cache(key, local_only=True) is the read that must survive a Redis failure
        self.internal_usage_cache.memory_get(&cache_key).await
    }

    // the Redis leg of _get_routed_model, with its failure reduced to a warning and a local read
    async fn redis_routed_model(&self, cache_key: &str) -> Option<String> {
        if !self.internal_usage_cache.redis_configured() {
            return None;
        }
        // redis_cache.async_get_cache(key) is the call whose exception falls back to in-memory
        let hit = match self.redis_hit(cache_key).await {
            Ok(hit) => hit,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "SensitiveDataRoutingHandler: Redis GET failed, falling back to in-memory"
                );
                return None;
            }
        };
        let (routed_model, ttl) = hit?;
        // async_set_cache(key, value, ttl, local_only=True) keeps a Redis hit in the local tier too
        self.internal_usage_cache
            .memory_set(cache_key, &routed_model, ttl)
            .await;
        Some(routed_model)
    }

    // a Redis hit and the ttl to copy it to the local tier with
    async fn redis_hit(&self, cache_key: &str) -> Result<Option<(String, Duration)>, C::Error> {
        let Some(routed_model) = self.internal_usage_cache.redis_get(cache_key).await? else {
            return Ok(None);
        };
        // async_get_ttl: None inherits the configured ttl, Err falls back to the local read
        let remaining = self.internal_usage_cache.redis_ttl(cache_key).await?;
        let ttl = remaining.unwrap_or_else(|| Duration::from_secs(self.ttl_seconds));
        Ok(Some((routed_model, ttl)))
    }

    // set_session_routing: Redis runs first, and a failure only logs because the local set follows
    pub async fn set_session_routing(
        &self,
        session_id: &str,
        model: &str,
        principal: PrincipalIdentity<'_>,
        guardrail_name: Option<&str>,
    ) {
        let tenant = resolve_tenant(principal);
        let cache_key = format!("{{{CACHE_PREFIX}{tenant}:{session_id}}}:model");
        tracing::info!(
            session_id,
            model,
            guardrail = guardrail_name,
            ttl = self.ttl_seconds,
            "SensitiveDataRoutingHandler: Setting session routing"
        );
        if self.internal_usage_cache.redis_configured() {
            // redis_cache.async_set_cache(key, value, ttl) is not optional, unlike the read fallback
            if let Err(error) = self
                .internal_usage_cache
                .redis_set(&cache_key, model, Duration::from_secs(self.ttl_seconds))
                .await
            {
                tracing::warn!(
                    error = %error,
                    "SensitiveDataRoutingHandler: Redis SET failed, falling back to in-memory"
                );
            }
        }
        // async_set_cache(key, value, ttl, local_only=True) always follows, even after a Redis failure
        self.internal_usage_cache
            .memory_set(&cache_key, model, Duration::from_secs(self.ttl_seconds))
            .await;
    }
}

// _resolve_tenant: the authenticated principal scopes the session, and an absent api_key falls to claims
fn resolve_tenant(principal: PrincipalIdentity<'_>) -> String {
    if let Some(api_key) = principal.hashed_api_key.filter(|key| !key.is_empty()) {
        return api_key.to_owned();
    }
    let claims = [
        ("user", principal.user_id),
        ("team", principal.team_id),
        ("org", principal.org_id),
    ];
    let mut scope = Vec::new();
    for (label, claim) in claims {
        let Some(claim_value) = claim.filter(|text| !text.is_empty()) else {
            continue;
        };
        scope.push(format!("{label}:{claim_value}"));
    }
    if scope.is_empty() {
        return "default".to_owned();
    }
    scope.join("|")
}
