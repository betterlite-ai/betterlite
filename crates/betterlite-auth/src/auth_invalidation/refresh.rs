use std::sync::Arc;

use betterlite_db::combined_key_read::view::epoch_decimal;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Serialize;
use tracing::warn;

use super::AuthInvalidation;
use super::error::InvalidationError;
use super::keys::InvalidationKey;
use super::ports::{DeferredDelete, RedisMarker};
use crate::auth_cache::copy::copy_for_cache;
use crate::auth_cache::payload;
use crate::resolved_key::auth::UserAPIKeyAuth;

impl AuthInvalidation {
    pub async fn cache_management_object<Value>(
        &self,
        key: &str,
        value: &Value,
    ) -> Result<(), InvalidationError>
    where
        Value: Serialize + Sync,
    {
        let payload =
            payload::encode(value).map_err(|error| InvalidationError::Payload(Box::new(error)))?;
        let ttl = Some(self.auth.management_ttl());
        self.auth.write_management(key, &payload, ttl).await
    }

    pub async fn cache_team_object(
        &self,
        team_id: &str,
        team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), InvalidationError> {
        let stamp = epoch_decimal(self.clock.now());
        let refreshed = stamp.to_f64().ok_or(InvalidationError::Timestamp(stamp))?;
        team.last_refreshed_at = Some(refreshed);
        let key = InvalidationKey::TeamId(team_id).render();
        let redis_shared = self.usage_redis_shared(&key);
        self.invalidate_usage_entry(&key, redis_shared, "team object")
            .await;
        self.cache_management_object(&key, team).await?;
        let alias = team.team.fields.base.team_alias.clone();
        let Some(alias) = alias.filter(|alias| !alias.is_empty()) else {
            return Ok(());
        };
        let alias_key = InvalidationKey::TeamAlias(&alias).render();
        self.delete_team_alias(&alias_key).await;
        self.invalidate_usage_entry(&alias_key, redis_shared, "team alias")
            .await;
        Ok(())
    }

    pub async fn cache_key_object(
        &self,
        hashed_token: &str,
        auth: &mut UserAPIKeyAuth,
    ) -> Result<(), InvalidationError> {
        auth.view.last_refreshed_at = Some(epoch_decimal(self.clock.now()));
        let cached = copy_for_cache(auth);
        self.cache_management_object(hashed_token, &cached).await
    }

    pub async fn cache_access_object(
        &self,
        access_group_id: &str,
        table: &AccessGroupTable,
    ) -> Result<(), InvalidationError> {
        let key = InvalidationKey::AccessGroupId(access_group_id).render();
        let payload =
            payload::encode(table).map_err(|error| InvalidationError::Payload(Box::new(error)))?;
        let ttl = Some(self.access_group_ttl);
        self.auth.write_management(&key, &payload, ttl).await
    }

    fn usage_redis_shared(&self, key: &str) -> bool {
        let Some(usage) = self.usage.as_ref() else {
            return false;
        };
        shared_handle(self.auth.redis_handle(key), usage.redis_handle())
    }

    async fn invalidate_usage_entry(&self, key: &str, redis_shared: bool, stale: &'static str) {
        let Some(usage) = self.usage.as_ref() else {
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

    async fn delete_team_alias(&self, alias_key: &str) {
        let outcome = match self.auth.delete_pre_call(alias_key).await {
            Ok(DeferredDelete::Settled) => Ok(()),
            Ok(DeferredDelete::Unqueued) => self.auth.delete_async(alias_key).await,
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
}

fn shared_handle(auth: Option<Arc<dyn RedisMarker>>, usage: Option<Arc<dyn RedisMarker>>) -> bool {
    match (auth, usage) {
        (Some(auth_handle), Some(usage_handle)) => Arc::ptr_eq(&auth_handle, &usage_handle),
        (None, None) => true,
        (Some(_auth_handle), None) | (None, Some(_usage_handle)) => false,
    }
}
