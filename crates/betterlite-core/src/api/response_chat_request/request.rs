use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use super::declared::declared_key;
use super::error::RequestError;
use super::extra_body::extract_extra_body_params;
use super::options::{effective_stream, map_optional_params};
use super::records::{ChatRequestInput, ResponsesChatRequest};
use crate::api::response_chat_history::history::{HistoryMessage, convert_history};

// CallTypes.responses.value in LiteLLM, which is also how CallType::Responses serializes
const RESPONSES_CALL_TYPE: &str = "responses";

struct RequestParts<'a> {
    model: &'a str,
    input_items: Vec<Value>,
    instructions: Option<String>,
    input_count: usize,
    params: Map<String, Value>,
    litellm_params: &'a IndexMap<String, Value>,
    headers: &'a IndexMap<String, String>,
}

pub fn build_request<'a, Client: ?Sized>(
    request: ChatRequestInput<'_>,
    logging: &'a mut LoggingState,
    client: Option<&'a Client>,
) -> Result<ResponsesChatRequest<'a, Client>, RequestError> {
    let ChatRequestInput {
        model,
        messages,
        optional_params,
        litellm_params,
        headers,
        reasoning_auto_summary,
    } = request;
    let (input_items, instructions) = converted_input(messages)?;
    let input_count = input_items.len();
    extract_extra_body_params(optional_params)?;
    let mut params = Map::new();
    map_optional_params(optional_params, &mut params, reasoning_auto_summary)?;
    let stream = effective_stream(optional_params, litellm_params)?;
    tracing::debug!(
        "Chat provider: Stream parameter: {}",
        if stream { "True" } else { "False" }
    );
    if stream {
        params.insert("stream".to_owned(), Value::Bool(true));
    }
    log_ignored_previous_response_id(optional_params);
    logging.call_type = RESPONSES_CALL_TYPE.to_owned();
    let parts = RequestParts {
        model,
        input_items,
        instructions,
        input_count,
        params,
        litellm_params,
        headers,
    };
    assemble(logging, client, parts)
}

fn converted_input(
    messages: &[HistoryMessage],
) -> Result<(Vec<Value>, Option<String>), RequestError> {
    let mut converted = convert_history(messages)?;
    if converted.input.is_empty() {
        // the Responses API rejects empty input, so a system-only chat carries the text as one system item
        if let Some(text) = converted.instructions.take() {
            let mut content = Map::new();
            content.insert("type".to_owned(), Value::String("input_text".to_owned()));
            content.insert("text".to_owned(), Value::String(text));
            let mut item = Map::new();
            item.insert("type".to_owned(), Value::String("message".to_owned()));
            item.insert("role".to_owned(), Value::String("system".to_owned()));
            item.insert(
                "content".to_owned(),
                Value::Array(vec![Value::Object(content)]),
            );
            converted.input = vec![Value::Object(item)];
        }
    }
    Ok((converted.input, converted.instructions))
}

fn log_ignored_previous_response_id(optional_params: &IndexMap<String, Value>) {
    if let Some(Value::String(id)) = optional_params.get("previous_response_id") {
        if !id.is_empty() {
            tracing::debug!(
                "Chat provider: Warning ignoring previous response ID: {}",
                id
            );
        }
    }
}

fn assemble<'a, Client: ?Sized>(
    logging: &'a mut LoggingState,
    client: Option<&'a Client>,
    parts: RequestParts<'_>,
) -> Result<ResponsesChatRequest<'a, Client>, RequestError> {
    // the flat request LiteLLM builds spreads the SDK params after model and input, so they win there
    let mut litellm_params = build_sanitized_litellm_params(parts.litellm_params);
    let model = litellm_params
        .shift_remove("model")
        .unwrap_or_else(|| Value::String(parts.model.to_owned()));
    let input = litellm_params
        .shift_remove("input")
        .unwrap_or(Value::Array(parts.input_items));
    let mut extra_headers = litellm_params
        .shift_remove("extra_headers")
        .map(|value| {
            serde_json::from_value(value).map_err(|_| RequestError::FieldType {
                field: "extra_headers",
                expected: "a map of strings",
            })
        })
        .transpose()?;
    if !parts.headers.is_empty() {
        extra_headers = Some(parts.headers.clone());
    }
    tracing::debug!(
        "Chat provider: Final request model={}, input_items={}",
        parts.model,
        parts.input_count
    );
    let params = merge_history_fields(parts.params, parts.instructions.as_deref());
    Ok(ResponsesChatRequest {
        params: typed_params(model, input, params)?,
        litellm_params,
        extra_headers,
        client,
        logging,
    })
}

fn merge_history_fields(
    mut params: Map<String, Value>,
    instructions: Option<&str>,
) -> Map<String, Value> {
    // LiteLLM's merge replaces any mapped instructions with the history's leading system text
    if let Some(text) = instructions.filter(|text| !text.is_empty()) {
        params.insert("instructions".to_owned(), Value::String(text.to_owned()));
    }
    // the Responses API allows 64 characters in user, so a longer string is shortened and others are left alone
    if let Some(Value::String(user)) = params.get("user").cloned() {
        if user.chars().count() > 64 {
            params.insert(
                "user".to_owned(),
                Value::String(user.chars().take(64).collect()),
            );
        }
    }
    params
}

fn build_sanitized_litellm_params(
    litellm_params: &IndexMap<String, Value>,
) -> IndexMap<String, Value> {
    let mut sanitized = IndexMap::with_capacity(litellm_params.len());
    for (key, value) in litellm_params {
        if !declared_key(key) {
            sanitized.insert(key.clone(), value.clone());
        }
    }
    let mut merged = Map::new();
    for name in ["metadata", "litellm_metadata"] {
        if let Some(Value::Object(fields)) = litellm_params.get(name) {
            for (key, value) in fields {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
    if merged.is_empty() {
        sanitized.shift_remove("litellm_metadata");
    } else {
        sanitized.insert("litellm_metadata".to_owned(), Value::Object(merged));
    }
    sanitized
}

fn typed_params(
    model: Value,
    input: Value,
    mut params: Map<String, Value>,
) -> Result<ResponsesApiRequestParams, RequestError> {
    params.insert("model".to_owned(), model);
    params.insert("input".to_owned(), input);
    let encoded = serde_json::to_vec(&params).map_err(|source| RequestError::Json { source })?;
    let mut decoder = serde_json::Deserializer::from_slice(&encoded);
    serde_path_to_error::deserialize(&mut decoder).map_err(|source| RequestError::Fields { source })
}
