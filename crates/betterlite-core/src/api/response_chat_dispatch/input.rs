use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Value;

use super::error::InputError;
use super::port::PostStreamDeps;
use crate::api::api_boundaries::ApiLimits;
use crate::api::response_chat_history::history::HistoryMessage;
use crate::api::response_chat_request::error::RequestError;
use crate::api::responses_input_content::is_truthy;

// The handler's typed inputs beside the caller's raw kwargs: the two objects the SDK boundary owns arrive built
pub struct BridgeCall<'call, Client: ?Sized> {
    pub kwargs: &'call IndexMap<String, Json>,
    pub model_response: ModelResponseSeed,
    pub logging: LoggingState,
    pub client: Option<&'call Client>,
    pub limits: ApiLimits,
    pub reasoning_auto_summary: bool,
    pub post_stream: Option<&'call PostStreamDeps<'call>>,
}

// The caller's ModelResponse keeps only these three fields after B919 rewrites the answer
pub struct ModelResponseSeed {
    pub id: String,
    pub created: i64,
    pub object: String,
}

impl Default for ModelResponseSeed {
    fn default() -> Self {
        Self {
            id: String::new(),
            created: 0,
            object: String::from("chat.completion"),
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
    let model = text(kwargs, "model")?.to_owned();
    let custom_llm_provider = text(kwargs, "custom_llm_provider")?.to_owned();
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

fn text<'call>(
    kwargs: &'call IndexMap<String, Json>,
    field: &'static str,
) -> Result<&'call str, InputError> {
    match kwargs.get(field) {
        Some(Json::Text(text)) => Ok(text),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(InputError::Required { field })
        }
    }
}

fn object<'call>(
    kwargs: &'call IndexMap<String, Json>,
    field: &'static str,
) -> Result<&'call IndexMap<String, Json>, InputError> {
    match kwargs.get(field) {
        Some(Json::Object(fields)) => Ok(fields),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(InputError::Required { field })
        }
    }
}

fn messages(kwargs: &IndexMap<String, Json>) -> Result<Vec<HistoryMessage>, InputError> {
    let items = match kwargs.get("messages") {
        Some(Json::List(items)) => items,
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(InputError::Required { field: "messages" });
        }
    };
    let value = serde_json::to_value(Json::List(items.clone())).map_err(RequestError::Json)?;
    serde_json::from_value(value)
        .map_err(RequestError::Json)
        .map_err(InputError::Request)
}

fn value_map(fields: &IndexMap<String, Json>) -> Result<IndexMap<String, Value>, InputError> {
    let value = serde_json::to_value(Json::Object(fields.clone())).map_err(RequestError::Json)?;
    serde_json::from_value(value)
        .map_err(RequestError::Json)
        .map_err(InputError::Request)
}

fn header_map(kwargs: &IndexMap<String, Json>) -> Result<IndexMap<String, String>, InputError> {
    let fields = object(kwargs, "headers")?;
    let value = serde_json::to_value(Json::Object(fields.clone())).map_err(RequestError::Json)?;
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
        Some(value) if !matches!(value, Json::Null) => json_truthy(value),
        Some(Json::Null) | None => litellm_params
            .get("stream")
            .is_some_and(|value| is_truthy(Some(value))),
    }
}

fn json_truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|float| float != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

// The iterator's json_mode only ever arrives as a bool from the caller
#[must_use]
pub fn json_mode(kwargs: &IndexMap<String, Json>) -> Option<bool> {
    match kwargs.get("json_mode") {
        Some(Json::Bool(flag)) => Some(*flag),
        Some(Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
