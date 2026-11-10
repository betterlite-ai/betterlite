pub mod error;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_models::decisions::Json;
use betterlite_models::provider_defaults_a::url::{parse, unparse};
use secrecy::SecretString;
use serde_json::Value;

use crate::chat::response::ChatResponse;
use crate::openai_base_config::{Headers, Object};

use self::error::HostModelIdentityError;
use self::reply::response_from_wire;

mod reply;
mod tests;

pub const MAX_REPLY_BYTES: usize = TransportLimits::DEFAULT.body_bytes;

pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &[
    "max_tokens",
    "max_completion_tokens",
    "response_format",
    "stream",
    "temperature",
    "top_p",
    "tool_choice",
    "tools",
    "presence_penalty",
    "frequency_penalty",
    "stream_options",
];

#[derive(Debug, Clone, Copy)]
pub struct PostCall<'request> {
    pub input: &'request Json,
    pub api_key: Option<&'request SecretString>,
    pub original_response: &'request str,
    pub complete_input_dict: &'request Object,
    pub status_code: u16,
    pub headers: &'request Headers,
}

pub trait PostCallLogger {
    fn post_call(&mut self, call: &PostCall<'_>);
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
    const fn host(self) -> &'static str {
        match self {
            Self::Clarifai { .. } => "clarifai",
            Self::Compactifai { .. } => "compactifai",
        }
    }
}

pub fn decode_host_named_reply(
    bytes: &[u8],
    mode: HostReplyMode<'_>,
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
    let model_is_absent = reply.get("model").is_none_or(Value::is_null);
    let mut response =
        response_from_wire(reply).map_err(|cause| HostModelIdentityError::Reply {
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
    call: PostCall<'_>,
    logger: &mut dyn PostCallLogger,
) -> Result<ChatResponse, HostModelIdentityError> {
    logger.post_call(&call);
    let decoded = decode_host_named_reply(call.original_response.as_bytes(), mode);
    match mode {
        HostReplyMode::Clarifai { .. } => decoded
            .map_err(|error| wrap_json_failure(error, call.status_code, call.headers.clone())),
        HostReplyMode::Compactifai { .. } => decoded,
    }
}

// ClarifaiConfig.get_base_model, whose None means the caller keeps the model it sent
pub fn clarifai_base_model(model: Option<&str>) -> Result<Option<String>, HostModelIdentityError> {
    match model.filter(|text| !text.is_empty()) {
        None => Ok(None),
        Some(text) => {
            let parts: Vec<&str> = text.split('.').collect();
            let [user_id, app_id, model_id] = parts.as_slice() else {
                return Err(HostModelIdentityError::ModelName {
                    model: text.to_owned(),
                });
            };
            Ok(Some(format!(
                "https://clarifai.com/{user_id}/{app_id}/models/{model_id}"
            )))
        }
    }
}

// ClarifaiConfig.transform_request hands the parent `get_base_model(model) or model`
pub fn clarifai_request_model(
    model: Option<&str>,
) -> Result<Option<String>, HostModelIdentityError> {
    let converted = clarifai_base_model(model)?;
    if converted.is_none() {
        return Ok(model.map(str::to_owned));
    }
    Ok(converted)
}

const LLMGW_PATH: &str = "/genai/llmgw/chat/completions";
const LLMGW_SUFFIX: &str = "/api/v2/genai/llmgw/chat/completions";

// DataRobotConfig._resolve_api_base edits only the path of the endpoint the caller configured
pub fn datarobot_api_base(api_base: &str) -> Result<String, HostModelIdentityError> {
    let mut parts = parse(api_base).map_err(|source| HostModelIdentityError::ApiBase {
        api_base: api_base.to_owned(),
        source,
    })?;
    let path = &parts.path;
    let mut resolved = if path.is_empty() || path == "/" {
        format!("{path}{LLMGW_SUFFIX}")
    } else if path.contains("api/v2")
        && !path.contains("api/v2/deployments")
        && !path.contains(LLMGW_PATH)
    {
        format!("{path}{LLMGW_PATH}")
    } else {
        path.clone()
    };
    if !resolved.ends_with('/') {
        resolved.push('/');
    }
    // one pass, so a longer run of slashes keeps the pairs a single pass leaves behind
    parts.path = resolved.replace("//", "/");
    Ok(unparse(&parts))
}
