use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_redis::batch::result::BatchResult;
use betterlite_redis::redis_cache::RedisCache;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::AuthCache;
use super::AuthCacheError;
use super::WriteOptions;
use super::backend::{AuthCacheTier, PreCall};
use super::decode::decode_auth_payload;
use super::keys::{Partition, partition};
use super::payload::{self, PayloadError, ReadOperation};
use crate::resolved_key::auth::{AuthError, UserAPIKeyAuth};

type AuthResult<T> = Result<Option<T>, AuthError>;
type PayloadResult<T> = Result<Option<T>, PayloadError>;

impl<T: AuthCacheTier> AuthCache<T> {
    pub(crate) fn tier(&self, key: &str) -> &T {
        match partition(key) {
            Partition::KeyObject => &self.key_objects,
            Partition::Ordinary => &self.ordinary,
        }
    }

    #[must_use]
    pub fn in_memory_cache_for(&self, key: &str) -> &Arc<T::Memory> {
        match partition(key) {
            Partition::KeyObject => self.key_objects.in_memory(),
            Partition::Ordinary => self.ordinary.in_memory(),
        }
    }

    pub fn update_cache_ttl(&self, memory_ttl: Option<f64>, redis_ttl: Option<f64>) {
        self.ordinary.update_cache_ttl(memory_ttl, redis_ttl);
        self.key_objects.update_cache_ttl(memory_ttl, redis_ttl);
    }

    pub fn update_in_memory_max_size(&self, max_size: Option<usize>) {
        self.ordinary.update_in_memory_max_size(max_size);
        self.key_objects.update_in_memory_max_size(max_size);
    }

    pub fn attach_redis_cache(&self, redis: Option<Arc<RedisCache>>, redis_ttl: Option<f64>) {
        self.ordinary.attach_redis_cache(redis.clone(), redis_ttl);
        self.key_objects.attach_redis_cache(redis, redis_ttl);
    }

    #[must_use]
    pub fn management_object_ttl(&self) -> f64 {
        self.ordinary
            .memory_ttl()
            .unwrap_or(self.management_ttl_fallback)
    }

    pub fn get(&self, key: &str, local_only: bool) -> Option<Json> {
        match partition(key) {
            Partition::KeyObject => self.key_objects.get(key, local_only),
            Partition::Ordinary => self.ordinary.get(key, local_only),
        }
    }

    pub async fn async_get(&self, key: &str, local_only: bool) -> Option<Json> {
        match partition(key) {
            Partition::KeyObject => self.key_objects.async_get(key, local_only).await,
            Partition::Ordinary => self.ordinary.async_get(key, local_only).await,
        }
    }

    pub fn get_typed<Model>(&self, key: &str, local_only: bool) -> PayloadResult<Model>
    where
        Model: DeserializeOwned,
    {
        self.get(key, local_only).map_or(Ok(None), |cached| {
            payload::decode_typed::<Model>(&cached, ReadOperation::Get)
        })
    }

    pub async fn async_get_typed<Model>(&self, key: &str, local_only: bool) -> PayloadResult<Model>
    where
        Model: DeserializeOwned,
    {
        let fetched = self.async_get(key, local_only).await;
        let Some(cached) = fetched else {
            return Ok(None);
        };
        payload::decode_typed::<Model>(&cached, ReadOperation::AsyncGet)
    }

    pub fn get_auth(&self, key: &str, local_only: bool) -> AuthResult<UserAPIKeyAuth> {
        self.get(key, local_only).map_or(Ok(None), |cached| {
            decode_auth_payload(&cached, ReadOperation::Get)
        })
    }

    pub async fn async_get_auth(&self, key: &str, local_only: bool) -> AuthResult<UserAPIKeyAuth> {
        self.async_get(key, local_only)
            .await
            .map_or(Ok(None), |cached| {
                decode_auth_payload(&cached, ReadOperation::AsyncGet)
            })
    }

    pub fn set<Value>(
        &self,
        key: Option<&str>,
        value: &Value,
        options: WriteOptions,
    ) -> Result<(), AuthCacheError>
    where
        Value: Serialize,
    {
        let encoded = payload::encode(value)?;
        if let Some(name) = key {
            self.tier(name)
                .set(Some(name), &encoded, options.cache_options());
        } else {
            self.ordinary.set(None, &encoded, options.cache_options());
        }
        Ok(())
    }

    pub async fn async_set<Value>(
        &self,
        key: Option<&str>,
        value: &Value,
        options: WriteOptions,
    ) -> Result<(), AuthCacheError>
    where
        Value: Serialize + Sync,
    {
        let encoded = payload::encode(value)?;
        let Some(name) = key else {
            // a null key still serialized, and reaches the ordinary partition here
            self.ordinary
                .async_set(None, &encoded, options.cache_options())
                .await;
            return Ok(());
        };
        self.async_write_named(name, &encoded, options).await
    }

    pub async fn async_set_payload<Model>(
        &self,
        key: &str,
        payload: &Json,
        options: WriteOptions,
    ) -> Result<(), AuthCacheError>
    where
        Model: DeserializeOwned + Serialize + Sync,
    {
        let encoded = payload::encode_validated::<Model>(payload)?;
        self.async_write_named(key, &encoded, options).await?;
        Ok(())
    }

    async fn async_write_named(
        &self,
        key: &str,
        payload: &Json,
        options: WriteOptions,
    ) -> Result<(), AuthCacheError> {
        let target = self.tier(key);
        if !options.local_only && !options.refresh_ttl {
            let queued = target.set_pre_call(key, payload, options.ttl.map(|ttl| ttl.seconds))?;
            if matches!(queued, PreCall::Queued) {
                return Ok(());
            }
        }
        target
            .async_set(Some(key), payload, options.cache_options())
            .await;
        Ok(())
    }

    pub async fn async_set_pipeline<Value>(
        &self,
        entries: &[(String, Value)],
        options: WriteOptions,
    ) -> Result<(), AuthCacheError>
    where
        Value: Serialize + Sync,
    {
        // every entry is serialized before either partition hears about the batch
        let mut normalized = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            normalized.push((key.clone(), payload::encode(value)?));
        }
        let (key_objects, ordinary): (Vec<_>, Vec<_>) = normalized
            .into_iter()
            .partition(|(key, _value)| partition(key) == Partition::KeyObject);
        if !key_objects.is_empty() {
            self.key_objects
                .async_set_pipeline(&key_objects, options.cache_options())
                .await?;
        }
        if !ordinary.is_empty() {
            self.ordinary
                .async_set_pipeline(&ordinary, options.cache_options())
                .await?;
        }
        Ok(())
    }

    pub fn delete(&self, key: &str) -> Result<(), AuthCacheError> {
        match self.tier(key).delete(key) {
            Ok(()) => Ok(()),
            Err(error) => Err(AuthCacheError::from(error)),
        }
    }

    pub async fn async_delete(&self, key: &str) -> Result<(), AuthCacheError> {
        match self.tier(key).async_delete(key).await {
            Ok(()) => Ok(()),
            Err(error) => Err(AuthCacheError::from(error)),
        }
    }

    pub fn delete_pre_call(&self, key: &str) -> Result<Option<BatchResult<()>>, AuthCacheError> {
        match self.tier(key).delete_pre_call(key) {
            Ok(queued) => Ok(queued),
            Err(error) => Err(AuthCacheError::from(error)),
        }
    }

    // both partitions are attempted at once, and the first failure in partition order is the one raised
    pub async fn async_delete_keys(&self, keys: &[String]) -> Result<(), AuthCacheError> {
        let (key_objects, ordinary): (Vec<String>, Vec<String>) = keys
            .iter()
            .cloned()
            .partition(|key| partition(key) == Partition::KeyObject);
        let (key_result, other_result) = tokio::join!(
            self.key_objects.async_delete_keys(&key_objects),
            self.ordinary.async_delete_keys(&ordinary),
        );
        if let Err(error) = key_result {
            return Err(AuthCacheError::from(error));
        }
        match other_result {
            Ok(()) => Ok(()),
            Err(error) => Err(AuthCacheError::from(error)),
        }
    }
}
