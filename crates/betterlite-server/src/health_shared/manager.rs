use std::ops::Sub as _;
use std::sync::Arc;
use std::time::Duration;

use betterlite_db::ptu::store::sleeper::Sleeper;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, Utc};
use num_traits::ToPrimitive as _;
use tokio_util::task::TaskTracker;

use crate::health_cycle::endpoints::HealthCheckResult;
use crate::model_deprecation::clock::UtcClock;

use super::cache::SharedHealthCache;
use super::cached::{
    CachedHealthCheck, cached_payload, encode_cached, epoch_seconds, from_cached, json_from_cached,
};
use super::error::SharedHealthError;
use super::lease::{release_lock, shared_lock_lease};
use super::probe::{SharedHealthProbe, SharedProbeCall};
use super::settings::SharedHealthSettings;
use super::status_view::{RedisStatus, SharedHealthStatus, cache_fields};

pub const POLL_INTERVAL_SECONDS: u64 = 5;

// pod_<wall-clock milliseconds> at initialization, as the source builds it.
#[must_use]
pub fn pod_id(now: DateTime<Utc>) -> String {
    let millis = now.timestamp_millis();
    format!("pod_{millis}")
}

// SharedHealthCheckManager: one Redis lock, one cached result and one pod id for the whole proxy.
pub struct SharedHealthCheckManager {
    pub redis: Option<Arc<dyn SharedHealthCache>>,
    pub health_check_ttl: i64,
    pub lock_ttl: i64,
    pub pod_id: String,
    pub clock: Arc<dyn UtcClock + Send + Sync>,
    pub tasks: TaskTracker,
}

impl SharedHealthCheckManager {
    pub const LOCK_KEY: &'static str = "health_check_lock";
    pub const CACHE_KEY: &'static str = "health_check_results";

    #[must_use]
    pub fn new(
        redis: Option<Arc<dyn SharedHealthCache>>,
        settings: SharedHealthSettings,
        clock: Arc<dyn UtcClock + Send + Sync>,
        tasks: TaskTracker,
    ) -> Self {
        Self {
            redis,
            health_check_ttl: settings.health_check_ttl,
            lock_ttl: settings.lock_ttl,
            pod_id: pod_id(clock.now()),
            clock,
            tasks,
        }
    }

    // async_set_cache(nx=True) stores the pod id: only the pod that wrote it may release it.
    pub async fn acquire_health_check_lock(&self) -> bool {
        let Some(redis) = self.redis.as_deref() else {
            tracing::debug!("redis_cache is None, skipping lock acquisition");
            return false;
        };
        match redis
            .set(Self::LOCK_KEY, &self.pod_id, self.lock_ttl, true)
            .await
        {
            Ok(true) => {
                tracing::info!(pod = %self.pod_id, "pod acquired the health check lock");
                true
            }
            Ok(false) => {
                tracing::debug!(pod = %self.pod_id, "pod failed to acquire the health check lock");
                false
            }
            Err(error) => {
                tracing::error!("Error acquiring health check lock: {error}");
                false
            }
        }
    }

    // The direct release a caller can await; the check itself holds the same release in a lease.
    pub async fn release_health_check_lock(&self) {
        let Some(redis) = self.redis.as_deref() else {
            return;
        };
        release_lock(redis, &self.pod_id, Self::LOCK_KEY).await;
    }

    pub async fn cached_health_check_results(&self) -> Option<CachedHealthCheck> {
        let redis = self.redis.as_deref()?;
        let stored = match redis.get(Self::CACHE_KEY).await {
            Ok(value) => value,
            Err(error) => {
                tracing::error!("Error getting cached health check results: {error}");
                return None;
            }
        };
        let value = stored?;
        let payload = match cached_payload(value) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::error!("Error getting cached health check results: {error}");
                return None;
            }
        };
        let ttl = self.health_check_ttl.to_f64().unwrap_or_default();
        if epoch_seconds(self.clock.now()).sub(payload.timestamp_seconds()) > ttl {
            tracing::debug!("Cached health check results expired");
            return None;
        }
        tracing::debug!("Using cached health check results");
        Some(payload)
    }

    // safe_dumps over the rows and the TTL: the in-process exception map is not part of the payload.
    pub async fn cache_health_check_results(
        &self,
        healthy: &[JsonObject],
        unhealthy: &[JsonObject],
    ) {
        let Some(redis) = self.redis.as_deref() else {
            return;
        };
        let timestamp = epoch_seconds(self.clock.now());
        let text = match encode_cached(healthy, unhealthy, timestamp, &self.pod_id) {
            Ok(text) => text,
            Err(error) => {
                tracing::error!("Error caching health check results: {error}");
                return;
            }
        };
        match redis
            .set(Self::CACHE_KEY, &text, self.health_check_ttl, false)
            .await
        {
            Ok(_stored) => tracing::info!(
                healthy = healthy.len(),
                unhealthy = unhealthy.len(),
                "cached health check results"
            ),
            Err(error) => tracing::error!("Error caching health check results: {error}"),
        }
    }

    // The five steps of perform_shared_health_check: cache, lock, run, cache again, poll or run locally.
    pub async fn perform_shared_health_check<S>(
        &self,
        call: SharedProbeCall<'_>,
        probe: &dyn SharedHealthProbe,
        sleeper: &mut S,
    ) -> Result<HealthCheckResult, SharedHealthError>
    where
        S: Sleeper + Send,
    {
        if let Some(cached) = self.cached_health_check_results().await {
            return Ok(from_cached(cached));
        }
        if self.acquire_health_check_lock().await {
            return self.run_checked(call, probe).await;
        }
        match self.redis.as_deref() {
            Some(redis) => self.wait_for_results(redis, call, probe, sleeper).await,
            None => probe.probe(call).await,
        }
    }

    // The lock holder's turn: run the check, cache what succeeded, and release the lock either way.
    async fn run_checked(
        &self,
        call: SharedProbeCall<'_>,
        probe: &dyn SharedHealthProbe,
    ) -> Result<HealthCheckResult, SharedHealthError> {
        let Some(redis) = self.redis.as_ref() else {
            return probe.probe(call).await;
        };
        let lease = shared_lock_lease(redis, &self.pod_id, Self::LOCK_KEY, &self.tasks);
        tracing::info!(
            pod = %self.pod_id,
            model_count = call.models.len(),
            "pod running the shared health check"
        );
        let outcome = probe.probe(call).await;
        if let Ok(result) = &outcome {
            self.cache_health_check_results(&result.healthy_endpoints, &result.unhealthy_endpoints)
                .await;
        }
        lease.release().await;
        outcome
    }

    // A waiting pod: sleep five seconds a turn and take the first fresh entry another pod cached.
    async fn wait_for_results<S>(
        &self,
        redis: &dyn SharedHealthCache,
        call: SharedProbeCall<'_>,
        probe: &dyn SharedHealthProbe,
        sleeper: &mut S,
    ) -> Result<HealthCheckResult, SharedHealthError>
    where
        S: Sleeper + Send,
    {
        tracing::debug!(pod = %self.pod_id, "pod waiting for another pod to complete the health check");
        let mut elapsed = 0_u64;
        let max_wait = u64::try_from(self.lock_ttl).unwrap_or_default();
        while elapsed < max_wait {
            sleeper
                .sleep(Duration::from_secs(POLL_INTERVAL_SECONDS))
                .await;
            elapsed = elapsed.saturating_add(POLL_INTERVAL_SECONDS);
            if let Some(cached) = self.cached_health_check_results().await {
                tracing::info!(
                    elapsed,
                    "pod using cached health check results after waiting"
                );
                return Ok(from_cached(cached));
            }
            if self.lock_released(redis).await {
                break;
            }
        }
        tracing::warn!(
            pod = %self.pod_id,
            elapsed,
            "pod falling back to a local health check after waiting"
        );
        probe.probe(call).await
    }

    // The lock read that ends the wait early; a failed read is not the same as a released lock.
    async fn lock_released(&self, redis: &dyn SharedHealthCache) -> bool {
        match redis.get(Self::LOCK_KEY).await {
            Ok(None) => {
                tracing::debug!(
                    "pod detected the lock released without a cache, stopping the wait"
                );
                true
            }
            Ok(Some(_owner)) => false,
            Err(error) => {
                tracing::debug!(%error, "the lock read failed, continuing the wait");
                false
            }
        }
    }

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
        let (cache_available, cache_age_seconds, last_checked_by) =
            cache_fields(self.cached_health_check_results().await, self.clock.now());
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
