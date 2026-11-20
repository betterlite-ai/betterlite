use std::sync::Arc;

use tracing::warn;

use super::AuthInvalidation;
use super::error::InvalidationError;
use super::ports::{DeferredDelete, RedisMarker};

pub(super) fn usage_redis_shared(invalidation: &AuthInvalidation, key: &str) -> bool {
    let Some(usage) = invalidation.usage.as_ref() else {
        return false;
    };
    shared_handle(invalidation.auth.redis_handle(key), usage.redis_handle())
}

pub(super) async fn invalidate_usage_entry(
    invalidation: &AuthInvalidation,
    key: &str,
    redis_shared: bool,
    stale: &'static str,
) {
    let Some(usage) = invalidation.usage.as_ref() else {
        return;
    };
    let outcome = if redis_shared {
        usage
            .delete_memory(key)
            .map_err(|source| InvalidationError::Memory(Box::new(source)))
    } else {
        usage.delete_async(key).await
    };
    if let Err(error) = outcome {
        let sanitized = key.replace(['\r', '\n'], "");
        warn!(
            subject = sanitized.as_str(),
            error = %error,
            "Failed to invalidate internal usage cache entry {sanitized}; a stale {stale} may be served until its TTL expires: {error}"
        );
    }
}

pub(super) async fn delete_team_alias(invalidation: &AuthInvalidation, alias_key: &str) {
    let outcome = match invalidation.auth.delete_pre_call(alias_key).await {
        Ok(DeferredDelete::Settled) => Ok(()),
        Ok(DeferredDelete::Unqueued) => invalidation.auth.delete_async(alias_key).await,
        Err(error) => Err(error),
    };
    if let Err(error) = outcome {
        warn!(
            subject = alias_key,
            error = %error,
            "Failed to invalidate cached team alias entry {alias_key}; a stale team object may be served until its TTL expires: {error}"
        );
    }
}

fn shared_handle(auth: Option<Arc<dyn RedisMarker>>, usage: Option<Arc<dyn RedisMarker>>) -> bool {
    match (auth, usage) {
        (Some(auth_handle), Some(usage_handle)) => Arc::ptr_eq(&auth_handle, &usage_handle),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}
