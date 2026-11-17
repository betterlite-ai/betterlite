use std::error::Error as StdError;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;

use crate::response_content::wire::{AnthropicCompletionResponse, JsonFields, get, json_text};
use crate::usage::ReasoningTokenCounter;

mod assemble;
pub mod error;
mod json_mode;
mod params;
mod tests;
pub mod wire;

use assemble::transform_parsed_response;
use error::{DecodeError, ResponseError};
use wire::{AnthropicReply, ResponseEnvelope};

pub struct RawResponse<'raw> {
    pub status: u16,
    pub headers: &'raw HeaderMap,
    pub body: &'raw [u8],
}

// the arguments LiteLLM's transform_response receives: the caller's request pieces plus the injected clock and flag
pub struct ResponseRequest<'request> {
    pub messages: &'request [Json],
    pub optional_params: &'request IndexMap<String, Json>,
    pub litellm_params: &'request IndexMap<String, Json>,
    pub request_data: &'request IndexMap<String, Json>,
    pub json_mode: Option<bool>,
    pub now: i64,
    pub disable_add_prefix_to_prompt: bool,
}

pub struct Logging<'logging> {
    pub state: &'logging mut LoggingState,
    pub env: &'logging PrePostEnv<'logging>,
}

// Python logs raw_response.text, which shows one U+FFFD for each invalid byte sequence
fn replaced_text(body: &[u8]) -> String {
    let mut text = String::with_capacity(body.len());
    for chunk in body.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}

fn post_request(request: &ResponseRequest<'_>, text: String) -> PostCall {
    PostCall {
        original_response: Json::Text(text),
        input: Json::List(request.messages.to_vec()),
        additional_args: AdditionalArgs {
            api_base: None,
            headers: None,
            complete_input_dict: Some(RawBody::Object(request.request_data.clone())),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    }
}

pub fn decode_anthropic_reply(body: &[u8]) -> Result<AnthropicReply, DecodeError> {
    let mut fields: JsonFields = serde_json::from_slice(body).map_err(DecodeError::from_json)?;
    if let Some(error) = fields.get("error") {
        return Ok(AnthropicReply::Error(error.clone()));
    }
    let Some(content) = fields.shift_remove("content") else {
        return Err(DecodeError::MissingContent);
    };
    let Json::List(blocks) = content else {
        return Err(DecodeError::ContentType);
    };
    Ok(AnthropicReply::Success(AnthropicCompletionResponse {
        content: blocks,
        fields,
    }))
}

pub fn transform_response<Counter>(
    envelope: &mut ResponseEnvelope,
    raw: &RawResponse<'_>,
    request: &ResponseRequest<'_>,
    logging: Logging<'_>,
    counter: &Counter,
) -> Result<(), ResponseError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let Logging { state, env } = logging;
    let text = replaced_text(raw.body);
    post_call(state, &post_request(request, text.clone()), env);
    match decode_anthropic_reply(raw.body) {
        Ok(reply) => transform_parsed_response(envelope, raw, request, reply, counter),
        Err(DecodeError::MalformedJson { source }) => Err(ResponseError::Anthropic {
            message: format!("Unable to get json response - {source}, Original Response: {text}"),
            status: raw.status,
            headers: raw.headers.clone(),
        }),
        Err(error) => Err(ResponseError::from(error)),
    }
}

// LiteLLM reads only the last message, and only when it is an assistant message that asks for a prefix
pub fn get_prefix_prompt<CounterFailure>(
    messages: &[Json],
) -> Result<Option<String>, ResponseError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let Some(message) = messages.last() else {
        return Ok(None);
    };
    match get(message, "role") {
        Some(Json::Text(role)) if role == "assistant" => {}
        Some(Json::Text(_)) => return Ok(None),
        Some(_) => {
            return Err(ResponseError::FieldType {
                field: "messages.role",
            });
        }
        None => return Err(ResponseError::MissingField("messages.role")),
    }
    if prefix_flag(message)? != Some(true) {
        return Ok(None);
    }
    Ok(get(message, "content")
        .and_then(json_text)
        .map(str::to_owned))
}

fn prefix_flag<CounterFailure>(
    message: &Json,
) -> Result<Option<bool>, ResponseError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    match get(message, "prefix") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(prefix)) => Ok(Some(*prefix)),
        Some(_) => Err(ResponseError::FieldType {
            field: "messages.prefix",
        }),
    }
}

// LiteLLM's get_error_class hands the message, status and headers to AnthropicError unchanged
#[must_use]
pub const fn get_error_class<CounterFailure>(
    message: String,
    status: u16,
    headers: HeaderMap,
) -> ResponseError<CounterFailure>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    ResponseError::Anthropic {
        message,
        status,
        headers,
    }
}
