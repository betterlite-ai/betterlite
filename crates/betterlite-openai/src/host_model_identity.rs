pub mod error;
pub mod identity;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde_json::Value;
use uuid::Uuid;

use crate::chat::response::ChatResponse;
use crate::openai_base_config::{Headers, Object};

use self::error::HostModelIdentityError;
use self::reply::response_from_wire;

mod reply;
mod tests;

pub const MAX_REPLY_BYTES: usize = TransportLimits::DEFAULT.body_bytes;

pub struct PostCall<'request> {
    pub input: &'request Json,
    pub api_key: Option<&'request SecretString>,
    pub original_response: &'request str,
    pub complete_input_dict: &'request Object,
}

pub trait PostCallLogger {
    fn post_call(&mut self, call: PostCall<'_>);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostReplyMode<'request> {
    Clarifai {
        request_model: &'request str,
    },
    Compactifai {
        request_model: &'request str,
        json_mode: bool,
    },
}

impl HostReplyMode<'_> {
    pub(crate) const fn host(self) -> &'static str {
        match self {
            Self::Clarifai { .. } => "clarifai",
            Self::Compactifai { .. } => "compactifai",
        }
    }
}

// ModelResponse invents an id and a creation time for a reply that carries neither
#[derive(Debug, Clone, Copy)]
pub struct ReplySource {
    pub identifier: fn() -> Uuid,
    pub clock: fn() -> i64,
}

#[derive(Debug, Clone, Copy)]
pub struct HostReply<'request> {
    pub body: &'request str,
    pub status_code: u16,
    pub headers: &'request Headers,
    pub messages: &'request Json,
    pub api_key: Option<&'request SecretString>,
    pub request_data: &'request Object,
    pub source: ReplySource,
}

pub fn decode_host_named_reply(
    bytes: &[u8],
    mode: HostReplyMode<'_>,
    source: &ReplySource,
) -> Result<ChatResponse, HostModelIdentityError> {
    let host = mode.host();
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(HostModelIdentityError::TooLarge);
    }
    let mut reply: Value =
        serde_json::from_slice(bytes).map_err(|cause| HostModelIdentityError::Json {
            host,
            source: cause,
        })?;
    if let HostReplyMode::Compactifai {
        json_mode: true, ..
    } = mode
    {
        convert_single_tool_calls(&mut reply).ok_or(HostModelIdentityError::Malformed { host })?;
    }
    let Some(fields) = reply.as_object() else {
        return Err(HostModelIdentityError::Malformed { host });
    };
    let model_is_absent = fields.get("model").is_none_or(Value::is_null);
    let mut response =
        response_from_wire(reply, source).map_err(|cause| HostModelIdentityError::Reply {
            host,
            source: cause,
        })?;
    match mode {
        HostReplyMode::Clarifai { request_model } => {
            if !model_is_absent {
                response.model = format!("clarifai/{request_model}");
            }
        }
        HostReplyMode::Compactifai { request_model, .. } => {
            response.model = format!("compactifai/{request_model}");
        }
    }
    Ok(response)
}

// CompactifAI's json mode moves one tool call's arguments into content, and an explicit null stays null
fn convert_single_tool_calls(reply: &mut Value) -> Option<()> {
    let choices = reply.get_mut("choices").and_then(Value::as_array_mut)?;
    for choice in choices {
        let choice_fields = choice.as_object_mut()?;
        let Some(message) = choice_fields.get_mut("message") else {
            continue;
        };
        if message.is_null() {
            continue;
        }
        let message_fields = message.as_object_mut()?;
        if message_fields.is_empty() {
            continue;
        }
        let Some(tool_calls) = message_fields.get_mut("tool_calls") else {
            continue;
        };
        if tool_calls.is_null() {
            continue;
        }
        let content = {
            let calls = tool_calls.as_array()?;
            if calls.len() != 1 {
                continue;
            }
            let call = calls.first().and_then(Value::as_object)?;
            let function = call.get("function").and_then(Value::as_object)?;
            function
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()))
        };
        message_fields.insert(String::from("content"), content);
        message_fields.insert(String::from("tool_calls"), Value::Null);
    }
    Some(())
}

fn log_reply(reply: &HostReply<'_>, logger: &mut dyn PostCallLogger) {
    logger.post_call(PostCall {
        input: reply.messages,
        api_key: reply.api_key,
        original_response: reply.body,
        complete_input_dict: reply.request_data,
    });
}

// only the json decode is inside the pinned try, so a shape failure keeps its own class
fn wrap_json_failure(
    failure: HostModelIdentityError,
    status_code: u16,
    headers: Headers,
) -> HostModelIdentityError {
    match failure {
        HostModelIdentityError::Json { source, .. } => HostModelIdentityError::Api {
            status_code,
            headers,
            message: format!("the Clarifai response body is not valid JSON: {source}"),
        },
        unchanged @ (HostModelIdentityError::TooLarge
        | HostModelIdentityError::Malformed { .. }
        | HostModelIdentityError::Reply { .. }
        | HostModelIdentityError::ModelName { .. }
        | HostModelIdentityError::ApiBase { .. }
        | HostModelIdentityError::Api { .. }) => unchanged,
    }
}

// both pinned transforms log the raw text before parsing; only Clarifai wraps a json failure
pub fn process_host_response(
    mode: HostReplyMode<'_>,
    reply: HostReply<'_>,
    logger: &mut dyn PostCallLogger,
) -> Result<ChatResponse, HostModelIdentityError> {
    log_reply(&reply, logger);
    let decoded = decode_host_named_reply(reply.body.as_bytes(), mode, &reply.source);
    match mode {
        HostReplyMode::Clarifai { .. } => decoded
            .map_err(|error| wrap_json_failure(error, reply.status_code, reply.headers.clone())),
        HostReplyMode::Compactifai { .. } => decoded,
    }
}
