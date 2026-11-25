use std::ops::Sub as _;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use serde::Serialize;

use super::cached::{epoch_seconds, json_from_cached};
use super::manager::SharedHealthCheckManager;

// get_health_check_status' dict: the four fixed keys, then whatever the configured Redis answers.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SharedHealthStatus {
    pub pod_id: String,
    pub redis_available: bool,
    pub lock_ttl: i64,
    pub cache_ttl: i64,
    #[serde(flatten)]
    pub redis: Option<RedisStatus>,
}

// The fields a configured Redis adds, in the order get_health_check_status sets them: the error alone when a read raised, otherwise the lock and cache facts.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum RedisStatus {
    Failed {
        error: String,
    },
    Available {
        lock_owner: Json,
        lock_in_progress: bool,
        cache_available: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_age_seconds: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        last_checked_by: Option<String>,
    },
}

// The route's status provider: whoever can answer get_health_check_status behind the auth stage.
#[async_trait]
pub trait SharedHealthStatusSource: Send + Sync {
    async fn status(&self) -> Result<SharedHealthStatus, SharedHealthStatusError>;
}

// A status call that raised before it could answer; the route renders it as LiteLLM's 500.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct SharedHealthStatusError {
    pub message: String,
}

#[async_trait]
impl SharedHealthStatusSource for SharedHealthCheckManager {
    async fn status(&self) -> Result<SharedHealthStatus, SharedHealthStatusError> {
        Ok(self.get_health_check_status().await)
    }
}

impl SharedHealthCheckManager {
    // get_health_check_status: the four fixed keys, then the lock and cache facts when Redis is configured.
    pub async fn get_health_check_status(&self) -> SharedHealthStatus {
        let mut status = SharedHealthStatus {
            pod_id: self.pod_id.clone(),
            redis_available: self.redis.is_some(),
            lock_ttl: self.lock_ttl,
            cache_ttl: self.health_check_ttl,
            redis: None,
        };
        let Some(redis) = self.redis.as_deref() else {
            return status;
        };
        let owner = match redis.get(Self::LOCK_KEY).await {
            Ok(owner) => owner,
            Err(error) => {
                status.redis = Some(RedisStatus::Failed {
                    error: error.to_string(),
                });
                return status;
            }
        };
        let lock_in_progress = owner.is_some();
        let lock_owner = owner.map_or(Json::Null, json_from_cached);
        let cached = self.cached_health_check_results().await;
        let cache_available = cached.is_some();
        let (cache_age_seconds, last_checked_by) = match cached {
            Some(payload) => (
                Some(epoch_seconds(self.clock.now()).sub(payload.timestamp)),
                payload.checked_by,
            ),
            None => (None, None),
        };
        status.redis = Some(RedisStatus::Available {
            lock_owner,
            lock_in_progress,
            cache_available,
            cache_age_seconds,
            last_checked_by,
        });
        status
    }
}
