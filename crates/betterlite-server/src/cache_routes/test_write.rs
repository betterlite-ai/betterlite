use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::backend::{BackendError, CacheBackend, CacheResult, Ttl};
use betterlite_cache::cache_key::request::{CacheKeyRequest, KeyParam};
use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::configuration::cache::{Backend, Cache};
use betterlite_cache::configuration::settings::DEFAULT_REDIS_TTL_SECONDS;
use betterlite_cache::envelope::backend::StoredBackend;
use betterlite_cache::envelope::facade::{Clock, EnvelopeSettings, ResponseCache};
use betterlite_cache::envelope::request::EnvelopeRequest;
use betterlite_cache::types::control::DynamicCacheControl;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;

use super::admin::CacheAdminError;

const TEST_RESULT: &str = "test_key";
const TEST_MODEL: &str = "test-model";
const TEST_ROLE: &str = "user";
const TEST_CONTENT: &str = "test from litellm";
// litellm.enable_caching_on_provider_specific_optional_params defaults to False
const INCLUDE_PROVIDER_PARAMS: bool = false;
// LiteLLM's add_cache writes with a plain SET, never SET NX
const REPLACE_EXISTING: bool = false;
const SET_VERBOSE: Payload = Payload::Bool(false);

// add_cache logs its failures through hooks; the proxy keeps only the error line.
struct ErrorLogger;

impl LoggerSink for ErrorLogger {
    fn debug(&self, _payload: &Payload) -> bool {
        false
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, payload: &Payload) -> bool {
        tracing::error!("{payload}");
        true
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

struct SilentConsole;

impl ConsoleSink for SilentConsole {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

// The envelope facade writes through this, so B4297's async_add_cache owns the key, envelope and batching.
struct RedisValues {
    cache: Arc<Cache>,
}

#[async_trait]
impl CacheBackend for RedisValues {
    fn default_ttl(&self) -> i64 {
        DEFAULT_REDIS_TTL_SECONDS
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        let Backend::Redis(redis) = &self.cache.backend else {
            return Err(BackendError::Unsupported("async_set_cache"));
        };
        log_write(
            redis
                .values
                .async_set_cache(Some(key), &value, ttl, REPLACE_EXISTING)
                .await,
        );
        Ok(())
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        let Backend::Redis(redis) = &self.cache.backend else {
            return Err(BackendError::Unsupported("batch_cache_write"));
        };
        log_write(redis.values.batch_cache_write(key, &value).await);
        Ok(())
    }
}

// LiteLLM's add_cache never raises: it logs the failure and the ping route still reports healthy.
fn log_write<Stored>(outcome: Result<Stored, RedisCacheValuesError>) {
    if let Err(source) = outcome {
        tracing::error!("LiteLLM Cache: exception in add_cache: {source}");
    }
}

// The envelope B4297's async_add_cache writes through, opened for the live cache with its flush size converted.
fn test_envelope(
    cache: &Arc<Cache>,
    clock: Clock,
) -> Result<ResponseCache<ErrorLogger, SilentConsole>, CacheAdminError> {
    let redis_flush_size = match cache.redis_flush_size {
        Some(size) => Some(
            i64::try_from(size).map_err(|source| CacheAdminError::FlushSize { size, source })?,
        ),
        None => None,
    };
    Ok(ResponseCache {
        backend: Arc::new(StoredBackend {
            backend: Arc::new(RedisValues {
                cache: Arc::clone(cache),
            }),
        }),
        key_settings: CacheKeySettings {
            cache_type: cache.cache_type,
            mode: cache.mode,
            namespace: cache.namespace.clone(),
            semantic_cache_scope: cache.semantic_cache_scope,
            include_provider_params: INCLUDE_PROVIDER_PARAMS,
        },
        settings: EnvelopeSettings {
            ttl: cache.ttl,
            redis_flush_size,
        },
        hooks: test_hooks()?,
        clock,
    })
}

// LiteLLM's /cache/ping test write: async_add_cache(result="test_key", model="test-model", messages=[...]).
pub(crate) async fn add_test_key(cache: Arc<Cache>, clock: Clock) -> Result<(), CacheAdminError> {
    let facade = test_envelope(&cache, clock)?;
    let mut request = EnvelopeRequest {
        cache_key: None,
        key: CacheKeyRequest {
            params: vec![
                KeyParam {
                    name: "model".to_owned(),
                    value: KeyValue::String(TEST_MODEL.to_owned()),
                },
                KeyParam {
                    name: "messages".to_owned(),
                    value: test_messages(),
                },
            ],
            metadata: None,
            litellm_metadata: None,
            litellm_params: None,
            cache_control: None,
        },
        control: DynamicCacheControl::default(),
    };
    facade
        .async_add_cache(
            None,
            &mut request,
            &CacheValue::Text(TEST_RESULT.to_owned()),
        )
        .await;
    Ok(())
}

fn test_hooks() -> Result<VerboseHooks<ErrorLogger, SilentConsole>, CacheAdminError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|source| CacheAdminError::TestWrite { source })?;
    Ok(VerboseHooks {
        logger: ErrorLogger,
        console: SilentConsole,
        set_verbose: SET_VERBOSE,
        redactor: Arc::new(redactor),
    })
}

fn test_messages() -> KeyValue {
    let mut message = KeyObject::new();
    message.insert("role".to_owned(), KeyValue::String(TEST_ROLE.to_owned()));
    message.insert(
        "content".to_owned(),
        KeyValue::String(TEST_CONTENT.to_owned()),
    );
    KeyValue::Array(vec![KeyValue::Object(message)])
}

mod tests;
