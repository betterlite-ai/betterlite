use betterlite_models::call_helpers::headers::{HeaderSource, process_response_headers};
use betterlite_models::decisions::{HiddenParams, Json as HiddenJson};
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use serde_json::Value;
use uuid::Uuid;

use crate::chat::response::ChatResponse;
use crate::chat::{FunctionCall, ToolCall, choice_finish_reason};
use crate::compatible_wire::error::CompatibleWireError;
use crate::compatible_wire::{ReplyOptions, decode_compatible_chat_reply};
use crate::fireworks_ai_chat::error::FireworksChatError;
use crate::fireworks_ai_chat::hidden::extract_hidden_params;
use crate::fireworks_ai_chat::tools::{FUNCTION, function_names};
use crate::fireworks_ai_chat::{JSON_TOOL_NAME, MODEL_PREFIX};
use crate::fireworks_ai_common::error::FireworksError;
use crate::openai_base_config::Headers;

const ARGUMENTS: &str = "arguments";
const PARAMETERS: &str = "parameters";
const NAME: &str = "name";
const MODEL: &str = "model";

pub struct ReplyInput<'request> {
    pub status: u16,
    pub headers: &'request IndexMap<String, String>,
    pub body: &'request [u8],
    pub tools: Option<&'request Json>,
    pub identifier: fn() -> Uuid,
    pub clock: fn() -> i64,
}

pub struct Reply {
    pub response: ChatResponse,
    pub hidden_params: HiddenParams,
}

// transform_response: ModelResponse(**completion_response), the model prefix and the content tool call repair
pub fn transform_response(input: &ReplyInput<'_>) -> Result<Reply, FireworksChatError> {
    let payload: Value =
        serde_json::from_slice(input.body).map_err(|source| not_json(input, &source))?;
    let fireworks = extract_hidden_params(&payload);
    // the shared decoder fills usage counts and an absent id, created and object the way ModelResponse does
    let options = ReplyOptions {
        json_mode: false,
        provider: None,
        base_model: None,
        identifier: input.identifier,
        clock: input.clock,
    };
    let mut response = decode_compatible_chat_reply(input.body, &options)
        .map_err(|source| reply_error(input, source))?
        .response;
    // LiteLLM prefixes a model the body names, and the empty string counts as named
    if payload.get(MODEL).is_some_and(Value::is_string) {
        response.model = format!("{MODEL_PREFIX}{}", response.model);
    }
    for choice in &mut response.choices {
        let (mapped, native) = choice_finish_reason(choice.finish_reason.as_deref());
        choice.finish_reason = Some(mapped);
        choice.provider_specific_fields = native;
        // Message() reads an empty tool_calls list as none, so the repair still sees the content
        if choice
            .message
            .tool_calls
            .as_ref()
            .is_some_and(Vec::is_empty)
        {
            choice.message.tool_calls = None;
        }
    }
    repair_content_tool_calls(&mut response, input.tools, input.identifier);
    let hidden_params = hidden_params(input.headers, fireworks);
    Ok(Reply {
        response,
        hidden_params,
    })
}

// raw_response.json() failed: FireworksAIException keeps the text, the status and the headers
fn not_json(input: &ReplyInput<'_>, source: &serde_json::Error) -> FireworksChatError {
    let text = std::str::from_utf8(input.body).unwrap_or_default();
    let message = format!("Unable to get json response - {source}, Original Response: {text}");
    let headers: Headers = input
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    FireworksChatError::Response(FireworksError {
        status_code: input.status,
        message,
        headers: Some(headers),
    })
}

// the same bytes already parsed as JSON, so a second JSON error only repeats the not-JSON one
fn reply_error(input: &ReplyInput<'_>, source: CompatibleWireError) -> FireworksChatError {
    match source {
        CompatibleWireError::Json(source) => not_json(input, &source),
        CompatibleWireError::Reply(source) => FireworksChatError::Reply { source },
    }
}

fn hidden_params(headers: &IndexMap<String, String>, fireworks: HiddenParams) -> HiddenParams {
    let additional = process_response_headers(HeaderSource::Upstream(headers));
    let mut hidden = HiddenParams::new();
    hidden.insert(
        String::from("additional_headers"),
        HiddenJson::Object(additional),
    );
    hidden.extend(fireworks);
    hidden
}

// _handle_message_content_with_tool_calls: only tools present, content present and tool_calls absent
fn repair_content_tool_calls(
    response: &mut ChatResponse,
    tools: Option<&Json>,
    identifier: fn() -> Uuid,
) {
    let Some(tools) = tools else {
        return;
    };
    if matches!(tools, Json::Null) {
        return;
    }
    let Some(names) = function_names(tools) else {
        return;
    };
    for choice in &mut response.choices {
        if choice.message.tool_calls.is_some() {
            continue;
        }
        let Some(content) = choice.message.content.as_deref() else {
            continue;
        };
        let Some(call) = content_tool_call(content, &names, identifier) else {
            continue;
        };
        choice.message.tool_calls = Some(vec![call]);
        choice.message.content = None;
    }
}

// a content that is not an object with a declared name stays content, and json_tool_call never converts
fn content_tool_call(
    content: &str,
    names: &[String],
    identifier: fn() -> Uuid,
) -> Option<ToolCall> {
    let parsed: Value = serde_json::from_str(content).ok()?;
    let fields = parsed.as_object()?;
    let name = fields.get(NAME)?.as_str()?;
    if name == JSON_TOOL_NAME || !names.iter().any(|declared| declared == name) {
        return None;
    }
    let arguments = match fields.get(ARGUMENTS) {
        None | Some(Value::Null) => match fields.get(PARAMETERS) {
            Some(Value::Object(parameters)) => Value::Object(parameters.clone()).to_string(),
            _ => String::new(),
        },
        Some(Value::String(text)) => text.clone(),
        Some(Value::Object(object)) => Value::Object(object.clone()).to_string(),
        Some(_) => return None,
    };
    Some(ToolCall {
        id: identifier().to_string(),
        kind: FUNCTION.to_owned(),
        function: FunctionCall {
            name: name.to_owned(),
            arguments,
        },
    })
}
