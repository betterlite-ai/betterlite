use std::fmt::Display;
use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::structured::Payload;
use serde::Deserialize;
use thiserror::Error;

use super::backend::EnvelopeBackend;
use super::decode::decode_cached;
use super::lookup::{CacheLookup, update_metadata};
use super::request::EnvelopeRequest;
use crate::backend::BackendError;
use crate::cache_key::derive::{get_cache_key, should_use_cache};
use crate::cache_key::error::CacheKeyError;
use crate::cache_key::settings::CacheKeySettings;
use crate::types::cache_type::LiteLlmCacheType;

mod write;

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;
// A per-call backend that replaces the facade's own one, LiteLLM's dynamic_cache_object.
pub type Dynamic<'backend> = Option<&'backend dyn EnvelopeBackend>;
pub type CachedResponse = Option<CacheValue>;

// The Cache fields the envelope functions read: the default ttl and the flush size that selects batched writes.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeSettings {
    pub ttl: Option<f64>,
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

struct PreparedRead {
    key: String,
    lookup: CacheLookup,
    max_age: Option<i64>,
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
    pub fn get_cache(&self, dynamic: Dynamic<'_>, request: &mut EnvelopeRequest) -> CachedResponse {
        let max_age = request.control.s_maxage.or(request.control.s_max_age); // s-maxage first
        let mut read = self.start_read(request, max_age)?;
        let backend = dynamic.unwrap_or_else(|| self.backend.as_ref());
        let outcome = backend.get_cache(&read.key, &mut read.lookup);
        self.complete_read(request, &read, outcome)
    }

    #[must_use]
    pub async fn async_get_cache(
        &self,
        dynamic: Dynamic<'_>,
        request: &mut EnvelopeRequest,
    ) -> CachedResponse {
        let max_age = request.control.s_max_age.or(request.control.s_maxage); // s-max-age first
        let mut read = self.start_read(request, max_age)?;
        let outcome = dynamic
            .unwrap_or_else(|| self.backend.as_ref())
            .async_get_cache(&read.key, &mut read.lookup)
            .await;
        self.complete_read(request, &read, outcome)
    }

    pub fn add_cache(&self, request: &mut EnvelopeRequest, result: &CacheValue) {
        if should_use_cache(&self.key_settings, request.key.cache_control.as_ref())
            && let Err(error) = self.set(request, result)
        {
            self.log_add_failure(&error);
        }
    }

    pub async fn async_add_cache(
        &self,
        dynamic: Dynamic<'_>,
        request: &mut EnvelopeRequest,
        result: &CacheValue,
    ) {
        if !should_use_cache(&self.key_settings, request.key.cache_control.as_ref()) {
            return;
        }
        let outcome = if self.key_settings.cache_type == LiteLlmCacheType::Redis
            && self.settings.redis_flush_size.is_some()
        {
            self.batch_cache_write(request, result).await
        } else {
            write::set_through(self, dynamic, request, result).await
        };
        if let Err(error) = outcome {
            self.log_add_failure(&error);
        }
    }

    pub async fn batch_cache_write(
        &self,
        request: &mut EnvelopeRequest,
        result: &CacheValue,
    ) -> Result<(), WriteError> {
        let prepared = write::prepare(self, request, result)?;
        self.backend
            .batch_cache_write(&prepared.key, prepared.envelope, prepared.ttl)
            .await?;
        Ok(())
    }

    fn set(&self, request: &mut EnvelopeRequest, result: &CacheValue) -> Result<(), WriteError> {
        let prepared = write::prepare(self, request, result)?;
        self.backend
            .set_cache(&prepared.key, prepared.envelope, prepared.ttl)?;
        Ok(())
    }

    fn start_read(
        &self,
        request: &mut EnvelopeRequest,
        max_age: Option<i64>,
    ) -> Option<PreparedRead> {
        if !should_use_cache(&self.key_settings, request.key.cache_control.as_ref()) {
            return None;
        }
        let key = match self.resolve_key(request) {
            Ok(key) => key,
            Err(error) => {
                let message = format!("An exception occurred: {error}");
                let _printed = self.hooks.main_print_verbose(Payload::String(message));
                return None;
            }
        };
        Some(PreparedRead {
            key,
            lookup: CacheLookup::project(&request.key),
            max_age,
        })
    }

    fn complete_read(
        &self,
        request: &mut EnvelopeRequest,
        read: &PreparedRead,
        outcome: Result<Option<CacheValue>, BackendError>,
    ) -> CachedResponse {
        let cached = match outcome {
            Ok(cached) => cached,
            Err(error) => {
                let message = format!("An exception occurred: {error}");
                let _printed = self.hooks.main_print_verbose(Payload::String(message));
                return None;
            }
        };
        update_metadata(&mut request.key, &read.lookup);
        match decode_cached(cached.as_ref(), read.max_age, (self.clock)()) {
            Ok(response) => response,
            Err(error) => {
                let message = format!("An exception occurred: {error}");
                let _printed = self.hooks.main_print_verbose(Payload::String(message));
                None
            }
        }
    }

    fn resolve_key(&self, request: &mut EnvelopeRequest) -> Result<String, CacheKeyError> {
        match request.cache_key.clone() {
            Some(key) => Ok(key),
            None => get_cache_key(&mut request.key, &self.key_settings, &self.hooks),
        }
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
