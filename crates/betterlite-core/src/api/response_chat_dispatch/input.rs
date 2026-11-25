use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::compatible_wire::{NEW_ID, system_clock};
use betterlite_streaming::stream_chunk_shaping::creation::{StreamInstant, system_instant};
use indexmap::IndexMap;
use serde_json::Value;
use uuid::Uuid;

use super::error::InputError;
use super::port::PostStreamDeps;
use crate::api::api_boundaries::ApiLimits;
use crate::api::response_chat_history::history::HistoryMessage;
use crate::api::response_chat_request::error::RequestError;
use crate::api::response_chat_result::records::ModelResponse;
use crate::api::responses_input_content::is_truthy;

// The handler's typed inputs beside the caller's raw kwargs: the two objects the SDK boundary owns arrive built
pub struct BridgeCall<'call, Client: ?Sized> {
    pub kwargs: &'call IndexMap<String, Json>,
    pub model_response: ModelResponse,
    pub logging: LoggingState,
    pub client: Option<&'call Client>,
    pub limits: ApiLimits,
    pub reasoning_auto_summary: bool,
    pub post_stream: Option<&'call PostStreamDeps<'call>>,
    pub stamps: StreamStamps,
}

// The clock and identifier sources the translator and wrapper stamp chunks with, injected so a caller controls them
#[derive(Clone, Copy)]
pub struct StreamStamps {
    pub now: fn() -> StreamInstant,
    pub next_id: fn() -> Uuid,
    pub clock: fn() -> i64,
}

impl Default for StreamStamps {
    fn default() -> Self {
        Self {
            now: system_instant,
            next_id: NEW_ID,
            clock: system_clock,
        }
    }
}

pub struct Input {
    pub model: String,
    pub custom_llm_provider: String,
    pub messages: Vec<HistoryMessage>,
    // the mapping reads the key order, so the caller's map is rebuilt in its own order
    pub optional_params: IndexMap<String, Json>,
    pub litellm_params: IndexMap<String, Value>,
    pub headers: IndexMap<String, String>,
}

// validate_input_kwargs: the first field that is absent or of another type is the one ValueError names
pub fn input(kwargs: &IndexMap<String, Json>) -> Result<Input, InputError> {
    let model = text(kwargs.get("model"), "model")?.to_owned();
    let custom_llm_provider =
        text(kwargs.get("custom_llm_provider"), "custom_llm_provider")?.to_owned();
    let messages = messages(kwargs)?;
    let optional_params = object(kwargs, "optional_params")?.clone();
    let litellm_params = value_map(object(kwargs, "litellm_params")?)?;
    let headers = header_map(kwargs)?;
    object(kwargs, "model_response")?;
    object(kwargs, "logging_obj")?;
    Ok(Input {
        model,
        custom_llm_provider,
        messages,
        optional_params,
        litellm_params,
        headers,
    })
}

fn text<'call>(value: Option<&'call Json>, field: &'static str) -> Result<&'call str, InputError> {
    if let Some(Json::Text(text)) = value {
        return Ok(text);
    }
    Err(InputError::Required { field })
}

fn object<'call>(
    kwargs: &'call IndexMap<String, Json>,
    field: &'static str,
) -> Result<&'call IndexMap<String, Json>, InputError> {
    let Some(Json::Object(fields)) = kwargs.get(field) else {
        return Err(InputError::Required { field });
    };
    Ok(fields)
}

fn messages(kwargs: &IndexMap<String, Json>) -> Result<Vec<HistoryMessage>, InputError> {
    let Some(Json::List(items)) = kwargs.get("messages") else {
        return Err(InputError::Required { field: "messages" });
    };
    let value = serde_json::to_value(Json::List(items.clone()))
        .map_err(|source| RequestError::Json { source })?;
    serde_json::from_value(value)
        .map_err(|source| RequestError::Json { source })
        .map_err(InputError::Request)
}

fn value_map(fields: &IndexMap<String, Json>) -> Result<IndexMap<String, Value>, InputError> {
    let value = serde_json::to_value(Json::Object(fields.clone()))
        .map_err(|source| RequestError::Json { source })?;
    serde_json::from_value(value)
        .map_err(|source| RequestError::Json { source })
        .map_err(InputError::Request)
}

fn header_map(kwargs: &IndexMap<String, Json>) -> Result<IndexMap<String, String>, InputError> {
    let value = serde_json::to_value(Json::Object(object(kwargs, "headers")?.clone()))
        .map_err(|source| RequestError::Json { source })?;
    serde_json::from_value(value).map_err(|_source| InputError::Field {
        field: "headers",
        expected: "a map of strings",
    })
}

// The outer stream kwarg only adds a stream key the caller's optional params do not already set
pub fn inject_stream(
    kwargs: &IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
) {
    let requested = matches!(kwargs.get("stream"), Some(Json::Bool(true)));
    if requested && !optional_params.contains_key("stream") {
        optional_params.insert(String::from("stream"), Json::Bool(true));
    }
}

// _resolve_stream_flag: a null optional value falls through to the SDK's stream, and bool() ends both
#[must_use]
pub fn resolve_stream_flag(
    optional_params: &IndexMap<String, Json>,
    litellm_params: &IndexMap<String, Value>,
) -> bool {
    match optional_params.get("stream") {
        Some(Json::Null) | None => litellm_params
            .get("stream")
            .is_some_and(|value| is_truthy(Some(value))),
        Some(value) => {
            serde_json::to_value(value).is_ok_and(|converted| is_truthy(Some(&converted)))
        }
    }
}

// The iterator's json_mode only ever arrives as a bool from the caller
#[must_use]
pub fn json_mode(kwargs: &IndexMap<String, Json>) -> Option<bool> {
    if let Some(Json::Bool(flag)) = kwargs.get("json_mode") {
        return Some(*flag);
    }
    None
}
