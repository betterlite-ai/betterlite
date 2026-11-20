use tracing::warn;

use super::AuthInvalidation;
use super::error::InvalidationError;
use crate::auth_pubsub::eviction::{CacheError, LocalEviction};
use crate::auth_pubsub::message::Invalidation;

pub(super) async fn evict_key(
    invalidation: &AuthInvalidation,
    hashed_token: &str,
) -> Result<(), InvalidationError> {
    let outcome = delete_auth_then_usage(invalidation, hashed_token).await;
    invalidation
        .transport
        .publish(hashed_token, None, None)
        .await;
    outcome
}

pub(super) async fn evict_then_publish(invalidation: &AuthInvalidation, key: &str) {
    if let Err(error) = delete_auth_then_usage(invalidation, key).await {
        warn!(
            subject = key,
            error = %error,
            "Failed to invalidate cached team entry {key} on delete; a deleted team may be served until its TTL expires: {error}"
        );
    }
    invalidation.transport.publish(key, None, None).await;
}

pub(super) async fn delete_auth_then_usage(
    invalidation: &AuthInvalidation,
    key: &str,
) -> Result<(), InvalidationError> {
    invalidation.auth.delete_sync(key)?;
    if let Some(usage) = invalidation.usage.as_ref() {
        usage.delete_async(key).await?;
    }
    Ok(())
}

impl LocalEviction for AuthInvalidation {
    fn apply(&self, invalidation: Invalidation) -> Result<(), CacheError> {
        if invalidation.new_value.is_some() {
            return Ok(());
        }
        let cache_key = invalidation.cache_key;
        self.auth
            .delete_memory(&cache_key)
            .map_err(|source| CacheError::Eviction {
                cache_key: cache_key.clone(),
                source,
            })?;
        match self.auth.delete_redis(&cache_key) {
            Ok(()) => Ok(()),
            Err(error) => {
                warn!(
                    subject = cache_key.as_str(),
                    error = %error,
                    "Failed to evict cached entry {cache_key}; a stale object may be served until its TTL expires: {error}"
                );
                Ok(())
            }
        }
    }
}
