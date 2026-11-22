use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_invalidation::AuthInvalidation;
use betterlite_auth::auth_invalidation::clock::SystemClock;
use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_auth::auth_invalidation::ports::{
    AuthInvalidationCache, DeferredDelete, RedisMarker, SpendCounterCache,
};
use betterlite_auth::auth_invalidation::transport::InvalidationTransport;
use betterlite_auth::auth_pubsub::eviction::LocalEviction;
use betterlite_cache::backend::BackendError;
use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;

// The ModelAccessGroupCache impl on AuthInvalidation takes a full AuthInvalidation, so its test needs no-op dependencies and a transport that records what the eviction broadcasts.
struct NoopAuth;

#[async_trait]
impl AuthInvalidationCache for NoopAuth {
    fn redis_handle(&self, _key: &str) -> Option<Arc<dyn RedisMarker>> {
        None
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }

    async fn write_management(
        &self,
        _key: &str,
        _payload: &Json,
        _ttl: Option<f64>,
    ) -> Result<(), InvalidationError> {
        Ok(())
    }

    fn delete_sync(&self, _key: &str) -> Result<(), InvalidationError> {
        Ok(())
    }

    async fn delete_async(&self, _key: &str) -> Result<(), InvalidationError> {
        Ok(())
    }

    async fn delete_pre_call(&self, _key: &str) -> Result<DeferredDelete, InvalidationError> {
        Ok(DeferredDelete::Unqueued)
    }

    fn delete_memory(&self, _key: &str) -> Result<(), BackendError> {
        Ok(())
    }

    fn delete_redis(&self, _key: &str) -> Result<(), RedisAdminError> {
        Ok(())
    }
}

struct NoopSpend;

#[async_trait]
impl SpendCounterCache for NoopSpend {
    fn redis_handle(&self) -> Option<Arc<dyn RedisMarker>> {
        None
    }

    fn set_memory(&self, _key: &str, _value: f64, _ttl: f64) -> Result<(), BackendError> {
        Ok(())
    }

    async fn set_redis(
        &self,
        _key: &str,
        _value: f64,
        _ttl: f64,
    ) -> Result<(), RedisCacheValuesError> {
        Ok(())
    }

    async fn delete_redis(&self, _key: &str) -> Result<(), RedisAdminError> {
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct RecordingTransport {
    broadcasts: Mutex<Vec<Vec<String>>>,
}

impl RecordingTransport {
    pub(crate) fn broadcasts(&self) -> Vec<Vec<String>> {
        let broadcasts = self.broadcasts.lock().expect("the recorded broadcasts");
        broadcasts.clone()
    }
}

#[async_trait]
impl InvalidationTransport for RecordingTransport {
    async fn publish(&self, _cache_key: &str, _new_value: Option<f64>, _ttl: Option<f64>) {}

    async fn evict_and_broadcast(&self, cache_keys: &[String], _local: &dyn LocalEviction) {
        self.broadcasts
            .lock()
            .expect("the recorded broadcasts")
            .push(cache_keys.to_vec());
    }
}

pub(crate) fn recording_auth_invalidation() -> (Arc<RecordingTransport>, AuthInvalidation) {
    let recorder = Arc::new(RecordingTransport::default());
    let transport = Arc::clone(&recorder);
    let invalidation = AuthInvalidation {
        auth: Arc::new(NoopAuth),
        usage: None,
        spend: Arc::new(NoopSpend),
        transport,
        clock: Arc::new(SystemClock),
        access_group_ttl: 60.0,
    };
    (recorder, invalidation)
}
