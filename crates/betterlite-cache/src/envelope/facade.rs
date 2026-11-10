use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::structured::Payload;
use serde::Deserialize;
use thiserror::Error;

use super::backend::EnvelopeBackend;
use super::decode::decode_cached;
use super::limits::{async_max_age, sync_max_age, write_ttl};
use super::lookup::{CacheLookup, update_metadata};
use super::request::EnvelopeRequest;
use crate::backend::BackendError;
use crate::cache_key::derive::{get_cache_key, should_use_cache};
use crate::cache_key::error::CacheKeyError;
use crate::cache_key::settings::CacheKeySettings;
use crate::types::cache_type::LiteLlmCacheType;

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

// The Cache fields the envelope functions read: the default ttl and the flush size that selects batched writes.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeSettings {
    pub ttl: Option<i64>,
    pub redis_flush_size: Option<i64>,
}

#[derive(Debug, Error)]
pub enum WriteError {
    #[error("the cache key could not be derived")]
    Key {
        #[from]
        source: CacheKeyError,
    },
    #[error("the cache write failed")]
    Backend {
        #[from]
        source: BackendError,
    },
}

pub struct ResponseCache<L: LoggerSink, C: ConsoleSink> {
    pub backend: Arc<dyn EnvelopeBackend>,
    pub key_settings: CacheKeySettings,
    pub settings: EnvelopeSettings,
    pub hooks: VerboseHooks<L, C>,
    pub clock: Clock,
}

impl<L, C> ResponseCache<L, C>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    #[must_use]
    pub fn get_cache(
        &self,
        dynamic: Option<&dyn EnvelopeBackend>,
        request: &mut EnvelopeRequest,
    ) -> Option<CacheValue> {
        if !should_use_cache(&self.key_settings, request.key.cache_control.as_ref()) {
            return None;
        }
        let key = match self.resolve_key(request) {
            Ok(key) => key,
            Err(error) => {
                self.log_exception(&error);
                return None;
            }
        };
        let mut lookup = CacheLookup::project(&request.key);
        let cached = match Self::lookup(dynamic, self.backend.as_ref(), &key, &mut lookup) {
            Ok(cached) => cached,
            Err(error) => {
                self.log_exception(&error);
                return None;
            }
        };
        update_metadata(&mut request.key, &lookup);
        match decode_cached(
            cached.as_ref(),
            sync_max_age(&request.control),
            (self.clock)(),
        ) {
            Ok(response) => response,
            Err(error) => {
                self.log_exception(&error);
                None
            }
        }
    }

    #[must_use]
    pub async fn async_get_cache(
        &self,
        dynamic: Option<&dyn EnvelopeBackend>,
        request: &mut EnvelopeRequest,
    ) -> Option<CacheValue> {
        if !should_use_cache(&self.key_settings, request.key.cache_control.as_ref()) {
            return None;
        }
        let key = match self.resolve_key(request) {
            Ok(key) => key,
            Err(error) => {
                self.log_exception(&error);
                return None;
            }
        };
        let mut lookup = CacheLookup::project(&request.key);
        let cached = match self.async_lookup(dynamic, &key, &mut lookup).await {
            Ok(cached) => cached,
            Err(error) => {
                self.log_exception(&error);
                return None;
            }
        };
        update_metadata(&mut request.key, &lookup);
        match decode_cached(
            cached.as_ref(),
            async_max_age(&request.control),
            (self.clock)(),
        ) {
            Ok(response) => response,
            Err(error) => {
                self.log_exception(&error);
                None
            }
        }
    }

    pub fn add_cache(&self, request: &mut EnvelopeRequest, result: &CacheValue) {
        if !should_use_cache(&self.key_settings, request.key.cache_control.as_ref()) {
            return;
        }
        if let Err(error) = self.write(request, result) {
            self.log_add_failure(&error);
        }
    }

    pub async fn async_add_cache(
        &self,
        dynamic: Option<&dyn EnvelopeBackend>,
        request: &mut EnvelopeRequest,
        result: &CacheValue,
    ) {
        if !should_use_cache(&self.key_settings, request.key.cache_control.as_ref()) {
            return;
        }
        if let Err(error) = self.async_write(dynamic, request, result).await {
            self.log_add_failure(&error);
        }
    }

    pub async fn batch_cache_write(
        &self,
        request: &mut EnvelopeRequest,
        result: &CacheValue,
    ) -> Result<(), WriteError> {
        let key = self.resolve_key(request)?;
        let envelope = self.envelope(result);
        let ttl = write_ttl(self.settings.ttl, &request.control)
            .and_then(|seconds| serde_json::Number::from(seconds).as_f64());
        self.backend.batch_cache_write(&key, envelope, ttl).await?;
        Ok(())
    }

    fn resolve_key(&self, request: &mut EnvelopeRequest) -> Result<String, CacheKeyError> {
        match request.cache_key.clone() {
            Some(key) => Ok(key),
            None => get_cache_key(&mut request.key, &self.key_settings, &self.hooks),
        }
    }

    fn lookup(
        dynamic: Option<&dyn EnvelopeBackend>,
        backend: &dyn EnvelopeBackend,
        key: &str,
        lookup: &mut CacheLookup,
    ) -> Result<Option<CacheValue>, BackendError> {
        match dynamic {
            Some(chosen) => chosen.get_cache(key, lookup),
            None => backend.get_cache(key, lookup),
        }
    }

    async fn async_lookup(
        &self,
        dynamic: Option<&dyn EnvelopeBackend>,
        key: &str,
        lookup: &mut CacheLookup,
    ) -> Result<Option<CacheValue>, BackendError> {
        match dynamic {
            Some(chosen) => chosen.async_get_cache(key, lookup).await,
            None => self.backend.async_get_cache(key, lookup).await,
        }
    }

    fn write(&self, request: &mut EnvelopeRequest, result: &CacheValue) -> Result<(), WriteError> {
        let key = self.resolve_key(request)?;
        let envelope = self.envelope(result);
        let ttl = write_ttl(self.settings.ttl, &request.control)
            .and_then(|seconds| serde_json::Number::from(seconds).as_f64());
        self.backend.set_cache(&key, envelope, ttl)?;
        Ok(())
    }

    async fn async_write(
        &self,
        dynamic: Option<&dyn EnvelopeBackend>,
        request: &mut EnvelopeRequest,
        result: &CacheValue,
    ) -> Result<(), WriteError> {
        if self.key_settings.cache_type == LiteLlmCacheType::Redis
            && self.settings.redis_flush_size.is_some()
        {
            return self.batch_cache_write(request, result).await;
        }
        let key = self.resolve_key(request)?;
        let envelope = self.envelope(result);
        let ttl = write_ttl(self.settings.ttl, &request.control)
            .and_then(|seconds| serde_json::Number::from(seconds).as_f64());
        match dynamic {
            Some(chosen) => chosen.async_set_cache(&key, envelope, ttl).await?,
            None => self.backend.async_set_cache(&key, envelope, ttl).await?,
        }
        Ok(())
    }

    fn envelope(&self, result: &CacheValue) -> CacheValue {
        let mut fields = BTreeMap::new();
        fields.insert(
            super::TIMESTAMP_FIELD.to_owned(),
            CacheValue::Float((self.clock)()),
        );
        fields.insert(super::RESPONSE_FIELD.to_owned(), result.clone());
        CacheValue::Object(fields)
    }

    fn log_exception<Failure>(&self, failure: &Failure)
    where
        Failure: Display,
    {
        let message = format!("An exception occurred: {failure}");
        let _printed = self.hooks.main_print_verbose(Payload::String(message));
    }

    fn log_add_failure<Failure>(&self, failure: &Failure)
    where
        Failure: Display,
    {
        let message = format!("LiteLLM Cache: exception in add_cache: {failure}");
        let _logged =
            self.hooks
                .utils_print_verbose(Payload::String(message), "ERROR", &Payload::Bool(true));
    }
}
