use std::io::Error as IoError;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers as OpenAiHeaders, OpenAiConfig, OpenAiSettings,
};
use http::HeaderValue;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Error as JsonError;

use crate::operation::{OperationError, success};

// LiteLLM's DEFAULT_CHAT_COMPLETION_PARAM_VALUES: these go through the OpenAI parameter policy, the rest are forwarded
const KNOWN_PARAMS: &str = "functions function_call temperature top_p n stream stream_options stop max_tokens max_completion_tokens modalities prediction audio presence_penalty frequency_penalty logit_bias user model custom_llm_provider response_format seed tools tool_choice max_retries logprobs top_logprobs extra_headers api_version parallel_tool_calls drop_params allowed_openai_params additional_drop_params messages reasoning_effort verbosity thinking web_search_options include_server_side_tool_invocations service_tier safety_identifier prompt_cache_key prompt_cache_retention store metadata context_management";
// LiteLLM reads these call kwargs itself, so they never reach the provider body
const REMOVED_KEYS: [&str; 5] = [
    "model",
    "stream",
    "mock_response",
    "mock_tool_calls",
    "stream_chunk_size",
];

pub(crate) fn chat_fields(
    request: &ChatRequest,
    model: &str,
    catalog: &CatalogCapabilities,
) -> Result<IndexMap<String, JsonValue>, OperationError> {
    let mut requested = request_fields(request, "chat")?;
    let messages = requested
        .shift_remove("messages")
        .unwrap_or(JsonValue::Array(Vec::new()));
    for key in REMOVED_KEYS {
        requested.shift_remove(key);
    }
    let (known, forwarded): (IndexMap<String, JsonValue>, IndexMap<String, JsonValue>) = requested
        .into_iter()
        .partition(|(name, _value)| KNOWN_PARAMS.split(' ').any(|known| known == name));
    let policy = OpenAiConfig {
        family: ConfigFamily::OpenAi,
        api_key: None,
        headers: OpenAiHeaders::new(),
        settings: OpenAiSettings::default(),
    };
    let supported = policy.supported_params(model, catalog);
    let mut optional: IndexMap<String, JsonValue> = IndexMap::new();
    for (name, value) in known {
        if supported.contains(&name.as_str()) {
            optional.insert(name, value);
        }
    }
    append_stream_options(request, &mut optional);
    optional.extend(forwarded);
    let mut body: IndexMap<String, JsonValue> = IndexMap::new();
    body.insert(String::from("model"), JsonValue::Text(model.to_owned()));
    body.insert(String::from("messages"), messages);
    body.extend(optional);
    Ok(body)
}

pub(crate) fn encode(fields: IndexMap<String, JsonValue>) -> Result<Vec<u8>, OperationError> {
    serde_json::to_vec(&JsonValue::Object(fields))
        .map_err(|source| OperationError::Encode { source })
}

fn append_stream_options(request: &ChatRequest, optional: &mut IndexMap<String, JsonValue>) {
    if !request.stream {
        return;
    }
    optional.insert(String::from("stream"), JsonValue::Bool(true));
    if optional.contains_key("stream_options") {
        return;
    }
    let mut stream_options = IndexMap::new();
    stream_options.insert(String::from("include_usage"), JsonValue::Bool(true));
    optional.insert(
        String::from("stream_options"),
        JsonValue::Object(stream_options),
    );
}

pub(crate) fn request_fields<T>(
    request: &T,
    label: &'static str,
) -> Result<IndexMap<String, JsonValue>, OperationError>
where
    T: Serialize,
{
    let text =
        serde_json::to_string(request).map_err(|source| OperationError::Encode { source })?;
    let value: JsonValue =
        serde_json::from_str(&text).map_err(|source| OperationError::Encode { source })?;
    let JsonValue::Object(fields) = value else {
        return Err(OperationError::Encode {
            source: JsonError::io(IoError::other(format!(
                "a {label} request did not serialize as an object"
            ))),
        });
    };
    Ok(fields)
}

pub(crate) fn parse<T>(
    response: UpstreamResponse,
    operation: &'static str,
) -> Result<T, OperationError>
where
    T: DeserializeOwned,
{
    let checked = success(response)?;
    serde_json::from_slice(&checked.body)
        .map_err(|source| OperationError::Decode { operation, source })
}

pub(crate) fn header_value(text: &str, name: &str) -> Result<HeaderValue, OperationError> {
    HeaderValue::try_from(text).map_err(|source| OperationError::InvalidHeader {
        name: name.to_owned(),
        source,
    })
}

// The credential is marked sensitive so the Debug text of a prepared request never prints it
pub(crate) fn authorization_value(text: &str) -> Result<HeaderValue, OperationError> {
    let mut value = header_value(text, "authorization")?;
    value.set_sensitive(true);
    Ok(value)
}
