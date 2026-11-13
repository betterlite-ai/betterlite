use std::time::{SystemTime, UNIX_EPOCH};

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
use uuid::Uuid;

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
        // append: HeaderMap matches names case-insensitively, so insert would drop one of two spellings
        map.append(HeaderName::try_from(name)?, HeaderValue::try_from(value)?);
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

pub struct ReplyOptions<'call> {
    pub json_mode: bool,
    pub provider: Option<&'call str>,
    pub base_model: Option<&'call str>,
    pub identifier: fn() -> Uuid,
    pub clock: fn() -> i64,
}

// ModelResponse names a reply it has to invent an id for with a chatcmpl prefix and a uuid4, as _generate_id does
pub const NEW_ID: fn() -> Uuid = Uuid::new_v4;

// ModelResponse fills created with int(time.time())
#[must_use]
pub fn system_clock() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
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
        additional_args: AdditionalArgs {
            api_base: None,
            headers: None,
            complete_input_dict: Some(input.data),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    post_call(state, &call, env);
    decode_compatible_chat_reply(bytes, options)
}

pub fn decode_compatible_chat_reply(
    bytes: &[u8],
    options: &ReplyOptions<'_>,
) -> Result<CompatibleReply, CompatibleWireError> {
    let mut reply: Value = serde_json::from_slice(bytes).map_err(CompatibleWireError::Json)?;
    clean_usage(&mut reply);
    if options.json_mode {
        convert_single_tool_calls(&mut reply);
    }
    fill_response_defaults(&mut reply, options.identifier, options.clock);
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

// a null usage is the absent one, which B004's response reads as zero counts
fn clean_usage(reply: &mut Value) {
    if reply.get("usage").is_some_and(Value::is_null)
        && let Some(fields) = reply.as_object_mut()
    {
        fields.remove("usage");
    }
    let Some(usage) = reply.get_mut("usage").and_then(Value::as_object_mut) else {
        return;
    };
    for (name, tokens) in usage.iter_mut() {
        if name.ends_with("_tokens") && tokens.is_null() {
            *tokens = Value::from(0_u64);
        }
    }
    // LiteLLM's Usage fills an absent count with zero, and B004's response cannot hold an absent one
    for name in ["prompt_tokens", "completion_tokens", "total_tokens"] {
        usage.entry(name.to_owned()).or_insert(Value::from(0_u64));
    }
}

fn convert_single_tool_calls(reply: &mut Value) {
    let Some(choices) = reply.get_mut("choices").and_then(Value::as_array_mut) else {
        return;
    };
    for choice in choices {
        if let Some(message) = choice.get_mut("message").and_then(Value::as_object_mut) {
            single_tool_call_to_content(message);
        }
    }
}

fn fill_response_defaults(reply: &mut Value, identifier: fn() -> Uuid, clock: fn() -> i64) {
    let Some(fields) = reply.as_object_mut() else {
        return;
    };
    // ModelResponse fills an absent id and created and always writes its own object, as B004's response has plain values
    if fields.get("model").is_none_or(Value::is_null) {
        fields.insert("model".to_owned(), Value::String(String::new()));
    }
    if fields.get("id").is_none_or(Value::is_null) {
        let id = format!("chatcmpl-{}", identifier());
        fields.insert("id".to_owned(), Value::String(id));
    }
    if fields.get("created").is_none_or(Value::is_null) {
        fields.insert("created".to_owned(), Value::from(clock()));
    }
    fields.insert(
        "object".to_owned(),
        Value::String("chat.completion".to_owned()),
    );
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
