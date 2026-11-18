use std::collections::BTreeMap;

use betterlite_db::key_user_writes::hash::hash_token;
use betterlite_models::decisions::Json;

use super::fill::{PrefetchError, read_redis_values, set_in_memory};
use super::memory::PrefetchCache;
use super::redis_rows::PrefetchRedis;
use crate::auth_cache::keys::CacheKey;

// litellm/constants.py: REGISTRY_ERROR_NEGATIVE_CACHE_TTL
pub const REGISTRY_ERROR_TTL: f64 = 30.0;

// a registry stored as a string is a sentinel, written with the shorter of the two registry TTLs
#[must_use]
pub fn identity_memory_ttl(value: &Json, management_ttl: f64) -> f64 {
    match value {
        Json::Text(_) => REGISTRY_ERROR_TTL.min(management_ttl),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            management_ttl
        }
    }
}

// all keys auth reads before it knows the key's owners; a resolved key skips the bearer hash
#[must_use]
pub fn identity_cache_keys(
    api_key: &str,
    end_user_id: Option<&str>,
    key_is_resolved: bool,
) -> Vec<String> {
    let mut keys = Vec::with_capacity(4);
    if !key_is_resolved {
        keys.push(hash_token(api_key));
    }
    if let Some(end_user_id) = end_user_id.filter(|id| !id.is_empty()) {
        keys.push(CacheKey::EndUserId(end_user_id).render());
        keys.push(CacheKey::EndUserRestrictedRegistry.render());
    }
    keys.push(CacheKey::ModelAccessGroupRegistry.render());
    keys
}

struct IdentityPlan {
    missing: Vec<String>,
    found: BTreeMap<String, Json>,
    management_ttl: f64,
}

// the keys' owner rows are absent from memory and fetched in one MGET; no database read happens here
pub struct IdentityPrefetch<'cache> {
    pub cache: &'cache dyn PrefetchCache,
}

impl IdentityPrefetch<'_> {
    // the best-effort catch covers the Redis read and the TTL selection; later memory writes propagate
    pub async fn prefetch(&self, cache_keys: &[String]) -> Result<(), PrefetchError> {
        let Some(redis) = self.cache.redis() else {
            return Ok(());
        };
        let planned = self.plan(cache_keys, redis.as_ref()).await;
        let plan = match planned {
            Ok(Some(plan)) => plan,
            Ok(None) => return Ok(()),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "auth identity prefetch skipped, falling back to per-key lookups"
                );
                return Ok(());
            }
        };
        for key in &plan.missing {
            if let Some(value) = plan.found.get(key) {
                let ttl = identity_memory_ttl(value, plan.management_ttl);
                let memory = self.cache.memory_for(key);
                set_in_memory(memory, key, value, Some(ttl))?;
            }
        }
        Ok(())
    }

    async fn plan(
        &self,
        cache_keys: &[String],
        redis: &dyn PrefetchRedis,
    ) -> Result<Option<IdentityPlan>, PrefetchError> {
        let mut missing: Vec<String> = Vec::new();
        for key in cache_keys {
            if missing.contains(key) || self.cache.memory_for(key).get(key).is_some() {
                continue;
            }
            missing.push(key.clone());
        }
        if missing.is_empty() {
            return Ok(None);
        }
        let found = read_redis_values(&missing, redis).await?;
        Ok(Some(IdentityPlan {
            missing,
            found,
            management_ttl: self.cache.management_ttl(),
        }))
    }
}
