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

fn request_parameters(hit: &CacheHitContext<'_>) -> Result<IndexMap<String, Json>, CacheHitError> {
    let preset_cache_key = preset_cache_key(hit)?;
    let mut parameters = IndexMap::new();
    parameters.insert(
        "logger_fn".to_owned(),
        hit.kwargs.get("logger_fn").cloned().unwrap_or(Json::Null),
    );
    parameters.insert("acompletion".to_owned(), Json::Bool(hit.is_async));
    parameters.insert("api_base".to_owned(), default_text(hit.kwargs, "api_base")?);
    parameters.insert(
        "metadata".to_owned(),
        default_object(hit.kwargs, "metadata")?,
    );
    parameters.insert(
        "model_info".to_owned(),
        default_object(hit.kwargs, "model_info")?,
    );
    parameters.insert(
        "proxy_server_request".to_owned(),
        optional_object(hit.kwargs, "proxy_server_request")?,
    );
    parameters.insert(
        "stream_response".to_owned(),
        default_object(hit.kwargs, "stream_response")?,
    );
    parameters.insert(
        "custom_llm_provider".to_owned(),
        hit.custom_llm_provider
            .map_or_else(|| Json::Null, |name| Json::Text(name.to_owned())),
    );
    parameters.insert(
        PRESET_CACHE_KEY.to_owned(),
        preset_cache_key
            .as_deref()
            .map_or_else(|| Json::Null, |key| Json::Text(key.to_owned())),
    );
    Ok(parameters)
}

fn call_parameters(hit: &CacheHitContext<'_>) -> Result<IndexMap<String, Json>, CacheHitError> {
    let mut parameters = IndexMap::new();
    parameters.insert("input".to_owned(), input_value(hit));
    // LiteLLM copies the request's api_key here; a secret never enters the logging state
    parameters.insert("api_key".to_owned(), Json::Null);
    parameters.insert("original_response".to_owned(), hit.cached_result.clone());
    parameters.insert("additional_args".to_owned(), Json::Null);
    parameters.insert("stream".to_owned(), Json::Bool(stream_flag(hit.kwargs)?));
    parameters.insert(
        "custom_llm_provider".to_owned(),
        hit.custom_llm_provider
            .map_or_else(|| Json::Null, |name| Json::Text(name.to_owned())),
    );
    Ok(parameters)
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
    match hit.kwargs.get(field) {
        None | Some(Json::Null) => Json::Text(String::new()),
        Some(value) => value.clone(),
    }
}

fn stream_flag(kwargs: &IndexMap<String, Json>) -> Result<bool, CacheHitError> {
    match kwargs.get("stream") {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(stream)) => Ok(*stream),
        Some(_) => Err(CacheHitError::FieldType { field: "stream" }),
    }
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

fn default_text(
    kwargs: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Json, CacheHitError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(Json::Text(String::new())),
        Some(Json::Text(value)) => Ok(Json::Text(value.clone())),
        Some(_) => Err(CacheHitError::FieldType { field }),
    }
}

fn default_object(
    kwargs: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Json, CacheHitError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(Json::Object(IndexMap::new())),
        Some(Json::Object(value)) => Ok(Json::Object(value.clone())),
        Some(_) => Err(CacheHitError::FieldType { field }),
    }
}

fn optional_object(
    kwargs: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Json, CacheHitError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(Json::Null),
        Some(Json::Object(value)) => Ok(Json::Object(value.clone())),
        Some(_) => Err(CacheHitError::FieldType { field }),
    }
}
