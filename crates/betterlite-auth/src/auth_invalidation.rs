pub mod clock;
pub mod error;
pub mod eviction;
pub mod keys;
pub mod member;
pub mod ports;
pub mod refresh;
pub mod registry;
pub mod transport;

mod tests;

use std::sync::Arc;

use betterlite_db::combined_key_read::view::epoch_decimal;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use futures_util::future::join_all;
use rust_decimal::{Decimal, prelude::ToPrimitive as _};
use serde::Serialize;
use tracing::warn;

use self::clock::InvalidationClock;
use self::error::InvalidationError;
use self::keys::InvalidationKey;
use self::ports::{AuthInvalidationCache, SpendCounterCache, UsageInvalidationCache};
use self::registry::MembershipInflight;
use self::transport::InvalidationTransport;
use crate::auth_cache::{copy::copy_for_cache, keys::CacheKey, payload};
use crate::resolved_key::auth::UserAPIKeyAuth;

// proxy_server.py's SPEND_DB_FLOOR_CACHE_TTL_SECONDS; the authoritative-floor reader reads the same value
pub const SPEND_DB_FLOOR_TTL_SECONDS: f64 = 5.0;
// the reset writes the live counter for a minute, the same window /key/{key}/reset_spend has used
pub const SPEND_COUNTER_TTL_SECONDS: f64 = 60.0;

pub struct AuthInvalidation {
    pub auth: Arc<dyn AuthInvalidationCache>,
    pub usage: Option<Arc<dyn UsageInvalidationCache>>,
    pub spend: Arc<dyn SpendCounterCache>,
    pub transport: Arc<dyn InvalidationTransport>,
    pub clock: Arc<dyn InvalidationClock>,
    pub access_group_ttl: f64,
}

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
        let redis_shared = refresh::usage_redis_shared(self, &key);
        refresh::invalidate_usage_entry(self, &key, redis_shared, "team object").await;
        self.cache_management_object(&key, team).await?;
        let alias = team.team.fields.base.team_alias.clone();
        let Some(team_alias) = alias.filter(|value| !value.is_empty()) else {
            return Ok(());
        };
        let alias_key = InvalidationKey::TeamAlias(&team_alias).render();
        refresh::delete_team_alias(self, &alias_key).await;
        refresh::invalidate_usage_entry(self, &alias_key, redis_shared, "team alias").await;
        Ok(())
    }

    pub async fn cache_key_object(
        &self,
        hashed_token: &str,
        auth: &mut UserAPIKeyAuth,
    ) -> Result<(), InvalidationError> {
        auth.view.last_refreshed_at = Some(epoch_decimal(self.clock.now()));
        self.cache_management_object(hashed_token, &copy_for_cache(auth))
            .await
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

    pub async fn invalidate_team_member_spend<Load>(
        &self,
        inflight: &MembershipInflight<Load>,
        user_id: &str,
        team_id: &str,
        new_spend: Option<Decimal>,
    ) -> Result<(), InvalidationError>
    where
        Load: Send + Sync,
    {
        let reservation_key = CacheKey::TeamMembershipReservation { user_id, team_id }.render();
        member::wait_for_membership_load(inflight, &reservation_key).await;
        if let Some(spend) = new_spend {
            member::reset_spend_counters(self, user_id, team_id, spend).await?;
        }
        let membership_keys = [
            CacheKey::TeamMembershipAuth { team_id, user_id }.render(),
            reservation_key,
        ];
        self.transport
            .evict_and_broadcast(&membership_keys, self)
            .await;
        Ok(())
    }

    pub async fn delete_cache_team_object(&self, team_id: &str, team_alias: Option<&str>) {
        let mut keys = vec![InvalidationKey::TeamId(team_id).render()];
        if let Some(alias) = team_alias.filter(|alias| !alias.is_empty()) {
            keys.push(InvalidationKey::TeamAlias(alias).render());
        }
        for key in keys {
            eviction::evict_then_publish(self, &key).await;
        }
    }

    pub async fn delete_cache_key_object(&self, hashed_token: &str) {
        if let Err(error) = eviction::evict_key(self, hashed_token).await {
            warn!(
                subject = hashed_token,
                error = %error,
                "Failed to invalidate cached key entry {hashed_token}; a stale key object may be served until its TTL expires: {error}"
            );
        }
    }

    pub async fn delete_cache_key_objects(&self, hashed_tokens: &[String]) {
        let requests = hashed_tokens
            .iter()
            .map(|token| eviction::evict_key(self, token));
        let outcomes = join_all(requests).await;
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
        eviction::delete_auth_then_usage(
            self,
            &InvalidationKey::AccessGroupId(access_group_id).render(),
        )
        .await
    }

    pub async fn delete_cached_project_object(&self, project_id: &str) {
        self.transport
            .evict_and_broadcast(&[CacheKey::ProjectId(project_id).render()], self)
            .await;
    }
}
