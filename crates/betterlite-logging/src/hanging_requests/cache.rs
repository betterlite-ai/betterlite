use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::alert_settings::HANGING_ALERT_BUFFER_SECONDS;
use crate::alert_settings::records::HangingRequestData;

#[async_trait]
pub trait HangingRequestCache: Send + Sync {
    async fn insert(&self, key: &str, value: &HangingRequestData, ttl: Duration);
    async fn get(&self, key: &str) -> Option<HangingRequestData>;
    async fn oldest_keys(&self, limit: usize) -> Vec<String>;
    fn remove(&self, key: &str);
    // litellm mutates the cached object in place, so the entry keeps its original expiry
    async fn replace(&self, key: &str, value: &HangingRequestData);
}

pub trait HangingCacheFactory: Send + Sync {
    fn create(&self, ttl: Duration) -> Arc<dyn HangingRequestCache>;
}

#[must_use]
pub fn hanging_cache_ttl(alerting_threshold_seconds: f64) -> Duration {
    let buffer = Duration::from_secs(HANGING_ALERT_BUFFER_SECONDS).as_secs_f64();
    let seconds = alerting_threshold_seconds.mul_add(1.5, buffer).trunc();
    if !seconds.is_finite() || seconds <= 0.0 {
        return Duration::ZERO;
    }
    Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX)
}
