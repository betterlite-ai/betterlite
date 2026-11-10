use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::Value;

use crate::chat::response::ChatResponse;
use crate::openai_base_config::{CatalogCapabilities, Headers, Object, OpenAiConfig};
use error::{CompatibleWireError, OpenAiLikeError};

pub mod error;
mod tests;

const MISSING_KEY: &str = "Missing API Key - A call is being made to LLM Provider but no key is set either in the environment variables ({LLM_PROVIDER}_API_KEY) or via params";
const MISSING_BASE: &str = "Missing API Base - A call is being made to LLM Provider but no api base is set either in the environment variables ({LLM_PROVIDER}_API_KEY) or via params";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointType {
    ChatCompletions,
    Embeddings,
}

pub fn validate_environment(
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    endpoint: EndpointType,
    explicit: Option<Headers>,
    custom_endpoint: bool,
) -> Result<(String, Headers), OpenAiLikeError> {
    if api_key.is_none() && explicit.is_none() {
        return Err(OpenAiLikeError {
            status_code: 400,
            message: MISSING_KEY.to_owned(),
        });
    }
    let Some(base) = api_base else {
        return Err(OpenAiLikeError {
            status_code: 400,
            message: MISSING_BASE.to_owned(),
        });
    };
    let mut headers = explicit.unwrap_or_else(|| {
        Headers::from([("Content-Type".to_owned(), "application/json".to_owned())])
    });
    if let Some(key) = api_key
        && !headers.contains_key("Authorization")
    {
        headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    let url = match (custom_endpoint, endpoint) {
        (true, _) => base.to_owned(),
        (false, EndpointType::ChatCompletions) => format!("{base}/chat/completions"),
        (false, EndpointType::Embeddings) => format!("{base}/embeddings"),
    };
    Ok((url, headers))
}

pub fn header_map(headers: &Headers) -> Result<HeaderMap, http::Error> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.insert(HeaderName::try_from(name)?, HeaderValue::try_from(value)?);
    }
    Ok(map)
}

pub struct ChatParams<'config> {
    pub config: &'config OpenAiConfig,
    pub catalog: &'config CatalogCapabilities,
    pub replace_max_completion_tokens: bool,
}

impl ChatParams<'_> {
    pub fn map(&self, model: &str, non_default: &Object, existing: &mut Object) {
        self.config
            .map_params(model, non_default, existing, self.catalog);
        if self.replace_max_completion_tokens
            && let Some(limit) = non_default.get("max_completion_tokens")
        {
            existing.insert("max_tokens".to_owned(), limit.clone());
            existing.remove("max_completion_tokens");
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ReplyOptions<'call> {
    pub json_mode: bool,
    pub provider: Option<&'call str>,
    pub base_model: Option<&'call str>,
}

// B004's response has no hidden-params field, so the hidden model travels beside it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibleReply {
    pub response: ChatResponse,
    pub hidden_model: Option<String>,
}

pub struct LogInput {
    pub messages: Json,
    pub data: RawBody,
}

#[must_use]
pub fn logged_args(api_base: Option<String>, body: Option<RawBody>) -> AdditionalArgs {
    AdditionalArgs {
        api_base,
        headers: None,
        complete_input_dict: body,
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}

pub fn transform_response(
    bytes: &[u8],
    options: &ReplyOptions<'_>,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    input: LogInput,
) -> Result<CompatibleReply, CompatibleWireError> {
    let call = PostCall {
        original_response: serde_json::from_slice(bytes).map_err(CompatibleWireError::Json)?,
        input: input.messages,
        additional_args: logged_args(None, Some(input.data)),
    };
    post_call(state, &call, env);
    decode_compatible_chat_reply(bytes, options)
}

pub fn decode_compatible_chat_reply(
    bytes: &[u8],
    options: &ReplyOptions<'_>,
) -> Result<CompatibleReply, CompatibleWireError> {
    let mut reply: Value = serde_json::from_slice(bytes).map_err(CompatibleWireError::Json)?;
    // a null usage is the absent one, which B004's response reads as zero counts
    if reply.get("usage").is_some_and(Value::is_null)
        && let Some(fields) = reply.as_object_mut()
    {
        fields.remove("usage");
    }
    if let Some(usage) = reply.get_mut("usage").and_then(Value::as_object_mut) {
        for (name, tokens) in usage.iter_mut() {
            if name.ends_with("_tokens") && tokens.is_null() {
                *tokens = Value::from(0_u64);
            }
        }
    }
    if options.json_mode
        && let Some(choices) = reply.get_mut("choices").and_then(Value::as_array_mut)
    {
        for choice in choices {
            if let Some(message) = choice.get_mut("message").and_then(Value::as_object_mut) {
                single_tool_call_to_content(message);
            }
        }
    }
    if let Some(fields) = reply.as_object_mut()
        && fields.get("model").is_none_or(Value::is_null)
    {
        fields.insert("model".to_owned(), Value::String(String::new()));
    }
    let mut response: ChatResponse =
        serde_json::from_value(reply).map_err(CompatibleWireError::Reply)?;
    if let Some(provider) = options.provider {
        response.model = format!("{provider}/{}", response.model);
    }
    Ok(CompatibleReply {
        response,
        hidden_model: options.base_model.map(str::to_owned),
    })
}

fn single_tool_call_to_content(message: &mut Object) {
    let Some([call]) = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
    else {
        return;
    };
    let Some(function) = call.get("function") else {
        return;
    };
    let arguments = function
        .get("arguments")
        .filter(|arguments| !arguments.is_null())
        .cloned()
        .unwrap_or_else(|| Value::String(String::new()));
    message.insert("content".to_owned(), arguments);
    message.insert("tool_calls".to_owned(), Value::Null);
}
