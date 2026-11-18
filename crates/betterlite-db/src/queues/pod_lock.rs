use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_redis::cache_primitives::decode::DecodeError;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use num_traits::ToPrimitive as _;
use uuid::Uuid;

use self::owner::{PodLockKey, PodOwnerError, decode_pod_owner};

pub mod cache;
pub mod owner;
mod tests;

// PodLockManager (litellm/proxy/db/db_transaction_queue/pod_lock_manager.py): one UUID owner per proxy,
// a Redis lease per cron job, and the acquired/released gauges of the service logger.

#[derive(Debug, thiserror::Error)]
pub enum LockCacheError {
    #[error("the redis cache {operation} failed")]
    Cache {
        operation: &'static str,
        #[source]
        source: RedisCacheError,
    },
    #[error("the redis cache {operation} failed")]
    Values {
        operation: &'static str,
        #[source]
        source: RedisCacheValuesError,
    },
    #[error("the redis cache {operation} failed")]
    Admin {
        operation: &'static str,
        #[source]
        source: RedisAdminError,
    },
    #[error("redis returned a reply that is not a deletion count for {operation}")]
    UnexpectedReply { operation: &'static str },
    #[error("the pod owner could not be written as JSON")]
    Encode(#[from] serde_json::Error),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Owner(#[from] PodOwnerError),
}

// The lease operations of one cache: no scripting, a cache that meets async_set_cache/async_get_cache/async_delete_cache, or one without.
pub trait LockCache: Send + Sync {
    fn get_owner(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, LockCacheError>> + Send;

    fn set_owner_if_absent(
        &self,
        key: &str,
        owner: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<bool, LockCacheError>> + Send;

    fn delete(&self, key: &str) -> impl Future<Output = Result<i64, LockCacheError>> + Send;

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>>;
}

// async_register_script's handle: _compare_and_delete_lock compares against the JSON-encoded owner stored by async_set_cache.
pub trait LockScript: Send + Sync {
    fn run(
        &self,
        key: &str,
        owner: &str,
    ) -> impl Future<Output = Result<bool, LockCacheError>> + Send;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PodLockCallType {
    Acquired,
    Released,
}

impl PodLockCallType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Acquired => "_emit_acquired_lock_event",
            Self::Released => "_emit_released_lock_event",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PodLockEvent {
    pub call_type: PodLockCallType,
    pub gauge_labels: String,
    pub gauge_value: f64,
    pub duration_seconds: f64,
}

// service_logger_obj.async_service_success_hook, behind a port so B012's sink and B191's delivery stay outside this crate.
pub trait PodLockEventSink: Send + Sync {
    fn emit(&self, event: PodLockEvent);
}

#[derive(Debug, Default)]
pub struct NoPodLockEventSink;

impl PodLockEventSink for NoPodLockEventSink {
    fn emit(&self, _event: PodLockEvent) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Acquisition {
    Fresh,
    Reentrant,
    Denied,
}

pub struct PodLockManager<C: LockCache> {
    pub pod_id: String,
    pub cache: Option<Arc<C>>,
    pub default_ttl_seconds: i64,
    pub event_duration_seconds: f64,
    pub events: Arc<dyn PodLockEventSink>,
    pub script: Mutex<Option<Arc<dyn LockScript>>>,
}

impl<C: LockCache> PodLockManager<C> {
    #[must_use]
    pub fn new(
        cache: Option<Arc<C>>,
        default_ttl_seconds: i64,
        events: Arc<dyn PodLockEventSink>,
    ) -> Self {
        Self {
            pod_id: Uuid::new_v4().to_string(),
            cache,
            default_ttl_seconds,
            event_duration_seconds: default_ttl_seconds.to_f64().unwrap_or_default(),
            events,
            script: Mutex::new(None),
        }
    }

    pub async fn acquire_lock(
        &self,
        cronjob_id: &str,
        ttl: Option<i64>,
        allow_reentrant: bool,
    ) -> Option<bool> {
        let Some(cache) = self.cache.as_deref() else {
            tracing::debug!("redis_cache is None, skipping acquire_lock");
            return None;
        };
        // ttl or DEFAULT_CRON_JOB_LOCK_TTL_SECONDS: a zero TTL is falsy in the source and falls back to the default.
        let lease_ttl = match ttl {
            Some(seconds) if seconds != 0 => seconds,
            _ => self.default_ttl_seconds,
        };
        let lock_key = PodLockKey(cronjob_id).to_string();
        match self
            .try_acquire(cache, &lock_key, lease_ttl, allow_reentrant)
            .await
        {
            Ok(Acquisition::Fresh) => {
                tracing::info!(
                    lock_key,
                    "Pod successfully acquired Redis lock for cronjob_id"
                );
                Some(true)
            }
            Ok(Acquisition::Reentrant) => {
                tracing::info!(lock_key, "Pod already holds the Redis lock for cronjob_id");
                self.emit(PodLockCallType::Acquired, cronjob_id);
                Some(true)
            }
            Ok(Acquisition::Denied) => Some(false),
            Err(error) => {
                tracing::error!(error = %error, lock_key, "Error acquiring Redis lock for cronjob_id");
                Some(false)
            }
        }
    }

    pub async fn release_lock(&self, cronjob_id: &str) {
        let Some(cache) = self.cache.as_deref() else {
            tracing::debug!("redis_cache is None, skipping release_lock");
            return;
        };
        let lock_key = PodLockKey(cronjob_id).to_string();
        tracing::debug!(
            lock_key,
            "Pod attempting to release Redis lock for cronjob_id"
        );
        match self.compare_and_delete(cache, &lock_key).await {
            Ok(true) => {
                tracing::info!(
                    lock_key,
                    "Pod successfully released Redis lock for cronjob_id"
                );
                self.emit(PodLockCallType::Released, cronjob_id);
            }
            Ok(false) => tracing::debug!(
                lock_key,
                "Pod failed to release Redis lock for cronjob_id (lock missing or held by another pod)"
            ),
            Err(error) => {
                tracing::error!(error = %error, lock_key, "Error releasing Redis lock for cronjob_id");
            }
        }
    }

    async fn try_acquire(
        &self,
        cache: &C,
        lock_key: &str,
        lease_ttl: i64,
        allow_reentrant: bool,
    ) -> Result<Acquisition, LockCacheError> {
        tracing::debug!(
            pod_id = %self.pod_id,
            lock_key,
            ttl = lease_ttl,
            "Pod attempting to acquire Redis lock for cronjob_id"
        );
        if cache
            .set_owner_if_absent(lock_key, &self.pod_id, lease_ttl)
            .await?
        {
            return Ok(Acquisition::Fresh);
        }
        let Some(bytes) = cache.get_owner(lock_key).await? else {
            return Ok(Acquisition::Denied);
        };
        let owner = decode_pod_owner(&bytes)?;
        if owner == self.pod_id && allow_reentrant {
            return Ok(Acquisition::Reentrant);
        }
        tracing::info!(
            pod_id = %self.pod_id,
            lock_key,
            holder = %owner,
            "Pod could not acquire Redis lock for cronjob_id, held by another pod"
        );
        Ok(Acquisition::Denied)
    }

    async fn compare_and_delete(&self, cache: &C, lock_key: &str) -> Result<bool, LockCacheError> {
        if let Some(script) = self.release_script(cache) {
            match script.run(lock_key, &self.pod_id).await {
                Ok(deleted) => return Ok(deleted),
                Err(_script_error) => {
                    self.clear_release_script();
                    tracing::warn!(
                        lock_key,
                        "Lua compare-and-delete failed for lock_key={lock_key}, falling back to GET+DEL"
                    );
                }
            }
        }
        // The fallback is not atomic: the owner is checked here and deleted below, so a reacquired lock can be removed between the two commands.
        let Some(bytes) = cache.get_owner(lock_key).await? else {
            return Ok(false);
        };
        let owner = decode_pod_owner(&bytes)?;
        if owner != self.pod_id {
            return Ok(false);
        }
        Ok(cache.delete(lock_key).await? == 1)
    }

    fn release_script(&self, cache: &C) -> Option<Arc<dyn LockScript>> {
        let mut cached = self.script.lock().unwrap_or_else(PoisonError::into_inner);
        if cached.is_none() {
            *cached = cache.compare_and_delete_script();
        }
        cached.clone()
    }

    fn clear_release_script(&self) {
        let mut cached = self.script.lock().unwrap_or_else(PoisonError::into_inner);
        *cached = None;
    }

    fn emit(&self, call_type: PodLockCallType, cronjob_id: &str) {
        self.events.emit(PodLockEvent {
            call_type,
            gauge_labels: format!("{cronjob_id}:{}", self.pod_id),
            gauge_value: match call_type {
                PodLockCallType::Acquired => 1.0,
                PodLockCallType::Released => 0.0,
            },
            duration_seconds: self.event_duration_seconds,
        });
    }
}
