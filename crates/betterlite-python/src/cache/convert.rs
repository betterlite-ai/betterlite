use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_cache::cache_key::derive::get_cache_key;
use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::codec::CacheCodec;
use betterlite_cache::disk::Clock;
use betterlite_cache::embedding::embedding_cache::EmbeddingCacheEntry;
use betterlite_cache::embedding::response::EmbeddingResponse;
use betterlite_cache::envelope::backend::EnvelopeBackend;
use betterlite_cache::envelope::facade::{EnvelopeSettings, ResponseCache};
use betterlite_cache::envelope::request::EnvelopeRequest;
use betterlite_cache::envelope::{RESPONSE_FIELD, TIMESTAMP_FIELD};
use betterlite_cache::handler::kwargs::{key_request, request_cache_key, request_control};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::VerboseHooks;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use serde::{Deserialize, Serialize};

use super::error::CacheError;
use super::settings::CacheSettingsWire;
use super::sink::{PythonConsole, PythonLog};

mod tests;

// One cache call's JSON: the call kwargs, and whichever of a result, a cached value or one embedding input the method takes.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CacheCall {
    pub(crate) kwargs: KeyObject,
    pub(crate) result: Option<Json>,
    pub(crate) value: Option<Json>,
    pub(crate) input: Option<KeyValue>,
    pub(crate) index: Option<usize>,
}

impl CacheCall {
    // LiteLLM's get_cache and _add_cache_logic pass a cache_key kwarg through and derive one only when it is absent.
    pub(crate) fn envelope_request(&self) -> Result<EnvelopeRequest, CacheError> {
        Ok(EnvelopeRequest {
            cache_key: request_cache_key(&self.kwargs)?,
            key: key_request(&self.kwargs)?,
            control: request_control(&self.kwargs)?,
        })
    }

    // ast-grep-ignore: no-padded-one-line-functions - every write path takes its result through here
    pub(crate) fn result_value(&self) -> Result<CacheValue, CacheError> {
        let result = self.result.as_ref().ok_or(CacheError::MissingResult)?;
        CacheCodec::encode(result).map_err(CacheError::from)
    }

    pub(crate) fn embedding_result(&self) -> Result<EmbeddingResponse, CacheError> {
        let result = self.result.as_ref().ok_or(CacheError::MissingResult)?;
        let text = serde_json::to_string(result).map_err(|source| CacheError::Encode { source })?;
        serde_json::from_str(&text).map_err(|source| CacheError::Result { source })
    }

    pub(crate) fn cached_value(&self) -> Result<Option<CacheValue>, CacheError> {
        match self.value.as_ref() {
            None | Some(Json::Null) => Ok(None),
            Some(value) => CacheCodec::encode(value)
                .map(Some)
                .map_err(CacheError::from),
        }
    }

    // ast-grep-ignore: no-one-line-functions
    pub(crate) fn input_value(&self) -> Result<&KeyValue, CacheError> {
        self.input.as_ref().ok_or(CacheError::MissingInput)
    }

    // ast-grep-ignore: no-one-line-functions - idx_in_result_data takes 0, as in LiteLLM's add_embedding_response_to_cache
    pub(crate) fn result_index(&self) -> usize {
        self.index.unwrap_or_default()
    }

    // ast-grep-ignore: no-padded-one-line-functions - s-maxage wins over s-max-age and neither bounds the age
    pub(crate) fn max_age(&self) -> Result<Option<i64>, CacheError> {
        let control = request_control(&self.kwargs)?;
        Ok(control.s_maxage.or(control.s_max_age))
    }
}

// ast-grep-ignore: no-one-line-functions
pub(crate) fn parse_settings(text: &str) -> Result<CacheSettingsWire, CacheError> {
    serde_json::from_str(text).map_err(|source| CacheError::Settings { source })
}

// ast-grep-ignore: no-one-line-functions
pub(crate) fn parse_call(text: &str) -> Result<CacheCall, CacheError> {
    serde_json::from_str(text).map_err(|source| CacheError::Request { source })
}

pub(crate) fn hooks() -> Result<VerboseHooks<PythonLog, PythonConsole>, CacheError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|source| CacheError::Redactor { source })?;
    Ok(VerboseHooks {
        logger: PythonLog,
        console: PythonConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    })
}

pub(crate) fn wall_clock() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// The envelope facade over one backend, from the settings and the effective ttl of the cache it belongs to.
pub(crate) fn facade(
    wire: &CacheSettingsWire,
    backend: Arc<dyn EnvelopeBackend>,
    ttl: Option<f64>,
    clock: Clock,
) -> Result<ResponseCache<PythonLog, PythonConsole>, CacheError> {
    Ok(ResponseCache {
        backend,
        key_settings: wire.key_settings(),
        settings: EnvelopeSettings {
            ttl,
            redis_flush_size: flush_size(wire.redis_flush_size)?,
        },
        hooks: hooks()?,
        clock,
    })
}

// A caller's own cache_key wins, as LiteLLM's get_cache and _add_cache_logic check that kwarg before deriving one.
pub(crate) fn resolve_key(
    request: &mut EnvelopeRequest,
    settings: &CacheKeySettings,
    hooks: &VerboseHooks<PythonLog, PythonConsole>,
) -> Result<String, CacheError> {
    match request.cache_key.clone() {
        Some(key) => Ok(key),
        None => get_cache_key(&mut request.key, settings, hooks).map_err(CacheError::from),
    }
}

// The cache dict's ttl wins over Cache.ttl, as LiteLLM's _add_cache_logic writes it into the backend kwargs.
pub(crate) fn request_ttl(request: &EnvelopeRequest, ttl: Option<f64>) -> Option<f64> {
    let control = request
        .control
        .ttl
        .and_then(|seconds| serde_json::Number::from(seconds).as_f64());
    control.or(ttl)
}

pub(crate) fn flush_size(size: Option<usize>) -> Result<Option<i64>, CacheError> {
    match size {
        Some(present) => i64::try_from(present)
            .map(Some)
            .map_err(|_range| CacheError::FlushSize { size: present }),
        None => Ok(None),
    }
}

// ast-grep-ignore: no-one-line-functions
pub(crate) fn value_text(value: &Option<CacheValue>) -> Result<String, CacheError> {
    serde_json::to_string(value).map_err(|source| CacheError::Encode { source })
}

// ast-grep-ignore: no-one-line-functions - a semantic lookup answers the response and its backend's similarity
pub(crate) fn semantic_text(
    value: &Option<CacheValue>,
    similarity: Option<f64>,
) -> Result<String, CacheError> {
    serde_json::to_string(&(value, similarity)).map_err(|source| CacheError::Encode { source })
}

pub(crate) fn envelope_text(
    key: &str,
    now: f64,
    result: &CacheValue,
    ttl: Option<f64>,
) -> Result<String, CacheError> {
    let mut cached_data = BTreeMap::new();
    cached_data.insert(TIMESTAMP_FIELD.to_owned(), CacheValue::Float(now));
    cached_data.insert(RESPONSE_FIELD.to_owned(), result.clone());
    let answer = PreparedWrite {
        cache_key: key,
        cached_data: CacheValue::Object(cached_data),
        ttl,
    };
    serde_json::to_string(&answer).map_err(|source| CacheError::Encode { source })
}

pub(crate) fn embedding_text(entry: &EmbeddingCacheEntry) -> Result<String, CacheError> {
    let answer = PreparedWrite {
        cache_key: &entry.cache_key,
        cached_data: entry.cached_data.clone(),
        ttl: entry.ttl,
    };
    serde_json::to_string(&answer).map_err(|source| CacheError::Encode { source })
}

// The envelope a Python backend stores, with the ttl it applies, so a dynamic cache object gets Rust's rules.
#[derive(Debug, Serialize)]
struct PreparedWrite<'value> {
    cache_key: &'value str,
    cached_data: CacheValue,
    ttl: Option<f64>,
}
