use std::ops::Sub as _;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::cached::{CachedHealthCheck, epoch_seconds};
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

// The cache facts get_health_check_status adds: availability, age from the payload's timestamp, and the checker.
#[must_use]
pub(crate) fn cache_fields(
    cached: Option<CachedHealthCheck>,
    now: DateTime<Utc>,
) -> (bool, Option<f64>, Option<String>) {
    match cached {
        Some(payload) => (
            true,
            Some(epoch_seconds(now).sub(payload.timestamp)),
            payload.checked_by,
        ),
        None => (false, None, None),
    }
}
