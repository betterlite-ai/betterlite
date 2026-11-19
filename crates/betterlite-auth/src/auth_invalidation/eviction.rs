use futures_util::future::join_all;
use tracing::warn;

use super::AuthInvalidation;
use super::error::InvalidationError;
use super::keys::InvalidationKey;
use crate::auth_cache::keys::CacheKey;
use crate::auth_pubsub::eviction::{CacheError, LocalEviction};
use crate::auth_pubsub::message::Invalidation;

impl AuthInvalidation {
    pub async fn delete_cache_team_object(&self, team_id: &str, team_alias: Option<&str>) {
        let mut keys = vec![InvalidationKey::TeamId(team_id).render()];
        if let Some(alias) = team_alias.filter(|alias| !alias.is_empty()) {
            keys.push(InvalidationKey::TeamAlias(alias).render());
        }
        for key in keys {
            self.evict_then_publish(&key).await;
        }
    }

    pub async fn delete_cache_key_object(&self, hashed_token: &str) {
        if let Err(error) = self.evict_key(hashed_token).await {
            warn!(
                subject = hashed_token,
                error = %error,
                "Failed to invalidate cached key entry {hashed_token}; a stale key object may be served until its TTL expires: {error}"
            );
        }
    }

    pub async fn delete_cache_key_objects(&self, hashed_tokens: &[String]) {
        let outcomes = join_all(hashed_tokens.iter().map(|token| self.evict_key(token))).await;
        for (hashed_token, outcome) in hashed_tokens.iter().zip(outcomes) {
            if let Err(error) = outcome {
                warn!(
                    subject = hashed_token.as_str(),
                    error = %error,
                    "Failed to evict cached key entry for {hashed_token}; a deleted key may authenticate until its TTL expires: {error}"
                );
            }
        }
    }

    pub async fn delete_cache_access_object(
        &self,
        access_group_id: &str,
    ) -> Result<(), InvalidationError> {
        self.delete_auth_then_usage(&InvalidationKey::AccessGroupId(access_group_id).render())
            .await
    }

    pub async fn delete_cached_project_object(&self, project_id: &str) {
        self.transport
            .evict_and_broadcast(&[CacheKey::ProjectId(project_id).render()], self)
            .await;
    }

    async fn evict_key(&self, hashed_token: &str) -> Result<(), InvalidationError> {
        let outcome = self.delete_auth_then_usage(hashed_token).await;
        self.transport.publish(hashed_token, None, None).await;
        outcome
    }

    async fn evict_then_publish(&self, key: &str) {
        if let Err(error) = self.delete_auth_then_usage(key).await {
            warn!(
                subject = key,
                error = %error,
                "Failed to invalidate cached team entry {key} on delete; a deleted team may be served until its TTL expires: {error}"
            );
        }
        self.transport.publish(key, None, None).await;
    }

    async fn delete_auth_then_usage(&self, key: &str) -> Result<(), InvalidationError> {
        self.auth.delete_sync(key)?;
        if let Some(usage) = self.usage.as_ref() {
            usage.delete_async(key).await?;
        }
        Ok(())
    }
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
