use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_environment_variables};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

// LiteLLM keeps this record on the logging object; betterlite's readers reach it through the call details
pub const CACHING_DETAILS: &str = "caching_details";
const LITELLM_PARAMS: &str = "litellm_params";
const PRESET_CACHE_KEY: &str = "preset_cache_key";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CachingDetails {
    pub cache_hit: Option<bool>,
    pub cache_duration_ms: Option<f64>,
}

// Everything LiteLLM's _update_litellm_logging_obj_environment receives besides the logging object
pub struct CacheHitContext<'hit> {
    pub model: &'hit str,
    pub custom_llm_provider: Option<&'hit str>,
    pub preset_cache_key: Option<&'hit str>,
    pub cache_duration_ms: Option<f64>,
    pub kwargs: &'hit IndexMap<String, Json>,
    pub cached_result: &'hit Json,
    pub is_async: bool,
    pub is_embedding: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CacheHitError {
    #[error("the {field} value has the wrong type")]
    FieldType { field: &'static str },
    #[error("the logging environment could not be updated")]
    Environment {
        #[source]
        source: LoggingStateError,
    },
}

impl From<&CachingDetails> for Json {
    fn from(details: &CachingDetails) -> Self {
        let mut record = IndexMap::new();
        record.insert(
            "cache_hit".to_owned(),
            details.cache_hit.map_or(Self::Null, Self::Bool),
        );
        record.insert(
            "cache_duration_ms".to_owned(),
            details
                .cache_duration_ms
                .and_then(serde_json::Number::from_f64)
                .map_or_else(|| Self::Null, Self::Number),
        );
        Self::Object(record)
    }
}

pub fn record_cache_hit_context(
    state: &mut LoggingState,
    hit: &CacheHitContext<'_>,
) -> Result<(), CacheHitError> {
    let parameters = request_parameters(hit)?;
    let update = CallUpdate {
        model: Some(hit.model.to_owned()),
        user: optional_text(hit.kwargs, "user")?,
        optional_params: IndexMap::new(),
        additional_params: call_parameters(hit)?,
    };
    update_environment_variables(state, &parameters, update)
        .map_err(|source| CacheHitError::Environment { source })?;
    let details = CachingDetails {
        cache_hit: Some(true),
        cache_duration_ms: hit.cache_duration_ms,
    };
    state
        .model_call_details
        .insert(CACHING_DETAILS.to_owned(), Json::from(&details));
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Declared {
    Text,
    Object,
    Boolean,
}

impl Declared {
    const fn holds(self, value: &Json) -> bool {
        match self {
            Self::Text => matches!(value, Json::Text(_)),
            Self::Object => matches!(value, Json::Object(_)),
            Self::Boolean => matches!(value, Json::Bool(_)),
        }
    }

    fn fallback(self) -> Json {
        match self {
            Self::Text => Json::Text(String::new()),
            Self::Object => Json::Object(IndexMap::new()),
            Self::Boolean => Json::Bool(false),
        }
    }
}

// LiteLLM reads these with kwargs.get(key, default): the default holds only when the key is absent
fn keyword<'kwargs>(
    kwargs: &'kwargs IndexMap<String, Json>,
    field: &'static str,
    declared: Declared,
) -> Result<Option<&'kwargs Json>, CacheHitError> {
    let Some(value) = kwargs.get(field) else {
        return Ok(None);
    };
    if matches!(value, Json::Null) || declared.holds(value) {
        return Ok(Some(value));
    }
    Err(CacheHitError::FieldType { field })
}

fn request_parameters(hit: &CacheHitContext<'_>) -> Result<IndexMap<String, Json>, CacheHitError> {
    let preset_cache_key = preset_cache_key(hit)?;
    let provider = hit
        .custom_llm_provider
        .map_or_else(|| Json::Null, |name| Json::Text(name.to_owned()));
    let mut parameters = IndexMap::new();
    parameters.insert(
        "logger_fn".to_owned(),
        hit.kwargs.get("logger_fn").cloned().unwrap_or(Json::Null),
    );
    parameters.insert("acompletion".to_owned(), Json::Bool(hit.is_async));
    for (field, declared) in [
        ("api_base", Declared::Text),
        ("metadata", Declared::Object),
        ("model_info", Declared::Object),
    ] {
        let value =
            keyword(hit.kwargs, field, declared)?.map_or_else(|| declared.fallback(), Json::clone);
        parameters.insert(field.to_owned(), value);
    }
    parameters.insert(
        "proxy_server_request".to_owned(),
        keyword(hit.kwargs, "proxy_server_request", Declared::Object)?
            .cloned()
            .unwrap_or(Json::Null),
    );
    parameters.insert(
        "stream_response".to_owned(),
        keyword(hit.kwargs, "stream_response", Declared::Object)?
            .map_or_else(|| Declared::Object.fallback(), Json::clone),
    );
    parameters.insert("custom_llm_provider".to_owned(), provider);
    parameters.insert(
        PRESET_CACHE_KEY.to_owned(),
        preset_cache_key.map_or(Json::Null, Json::Text),
    );
    Ok(parameters)
}

fn call_parameters(hit: &CacheHitContext<'_>) -> Result<IndexMap<String, Json>, CacheHitError> {
    let stream = keyword(hit.kwargs, "stream", Declared::Boolean)?
        .map_or_else(|| Declared::Boolean.fallback(), Json::clone);
    let provider = hit
        .custom_llm_provider
        .map_or_else(|| Json::Null, |name| Json::Text(name.to_owned()));
    // LiteLLM copies the request's api_key here; a secret never enters the logging state
    Ok(IndexMap::from([
        ("input".to_owned(), input_value(hit)),
        ("api_key".to_owned(), Json::Null),
        ("original_response".to_owned(), hit.cached_result.clone()),
        ("additional_args".to_owned(), Json::Null),
        ("stream".to_owned(), stream),
        ("custom_llm_provider".to_owned(), provider),
    ]))
}

fn preset_cache_key(hit: &CacheHitContext<'_>) -> Result<Option<String>, CacheHitError> {
    if let Some(key) = hit.preset_cache_key {
        return Ok(Some(key.to_owned()));
    }
    let parameters = match hit.kwargs.get(LITELLM_PARAMS) {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Object(parameters)) => parameters,
        Some(_) => {
            return Err(CacheHitError::FieldType {
                field: LITELLM_PARAMS,
            });
        }
    };
    match parameters.get(PRESET_CACHE_KEY) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(key)) => Ok(Some(key.clone())),
        Some(_) => Err(CacheHitError::FieldType {
            field: PRESET_CACHE_KEY,
        }),
    }
}

fn input_value(hit: &CacheHitContext<'_>) -> Json {
    let field = if hit.is_embedding {
        "input"
    } else {
        "messages"
    };
    hit.kwargs
        .get(field)
        .cloned()
        .unwrap_or_else(|| Json::Text(String::new()))
}

fn optional_text(
    kwargs: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<String>, CacheHitError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(_) => Err(CacheHitError::FieldType { field }),
    }
}
