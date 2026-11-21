use betterlite_openai::openai_base_config::{ModelFamily, azure_model_family};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use serde_json::Number;

use crate::azure_chat_base_policy::messages::transform_request as base_transform;
use crate::chat_params::gpt5::transform_request as gpt5_transform;

use super::error::AzureChatError;

pub const CLOUDFLARE_MARKER: &str = "gateway.ai.cloudflare.com";
pub const MAX_RETRIES_KEY: &str = "max_retries";
pub const JSON_MODE_KEY: &str = "json_mode";
pub const EXTRA_HEADERS_KEY: &str = "extra_headers";
pub const STREAM_KEY: &str = "stream";

// optional_params.pop("max_retries", None), then DEFAULT_MAX_RETRIES when it is absent or None
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retries(pub Json);

impl Retries {
    #[must_use]
    pub fn pop(optional: &mut Object, default: i64) -> Self {
        Self(
            optional
                .shift_remove(MAX_RETRIES_KEY)
                .unwrap_or_else(|| Json::Number(Number::from(default))),
        )
    }

    // isinstance(value, int) accepts a bool, which Python's JSON decoder builds for true and false
    #[must_use]
    pub fn is_int(&self) -> bool {
        match &self.0 {
            Json::Bool(_) => true,
            Json::Number(number) => number.as_i64().is_some() || number.as_u64().is_some(),
            Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => false,
        }
    }

    // the streaming helpers take an int and never read it; a non-int keeps the configured default
    #[must_use]
    pub fn count(&self, fallback: i64) -> i64 {
        match &self.0 {
            Json::Bool(true) => 1,
            Json::Bool(false) => 0,
            Json::Number(number) => number.as_i64().unwrap_or(fallback),
            Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => fallback,
        }
    }
}

// if headers: optional_params["extra_headers"] = headers
pub fn inject_headers(optional: &mut Object, headers: &SecretHeaders) {
    let fields: Object = headers
        .0
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    optional.insert(EXTRA_HEADERS_KEY.to_owned(), Json::Object(fields));
}

// The request body: the Cloudflare branch sends the deployment in the url, so its model is null
pub fn build_data(
    model: &str,
    messages: &[Message],
    optional: &Object,
    base_model: Option<&str>,
    cloudflare: bool,
    openai_system_messages_first: bool,
) -> Result<Object, AzureChatError> {
    if cloudflare {
        return cloudflare_data(messages, optional);
    }
    let selected = base_model.unwrap_or(model);
    let transformed = if azure_model_family(selected) == ModelFamily::GptReasoning {
        gpt5_transform(model, messages, optional, openai_system_messages_first)
    } else {
        base_transform(model, messages, optional, openai_system_messages_first)
    };
    transformed.map_err(|error| {
        AzureChatError::azure(error.status_code().unwrap_or(500), error.to_string())
    })
}

fn cloudflare_data(messages: &[Message], optional: &Object) -> Result<Object, AzureChatError> {
    let mut data = Object::new();
    data.insert("model".to_owned(), Json::Null);
    data.insert("messages".to_owned(), message_list(messages)?);
    for (key, value) in optional {
        data.insert(key.clone(), value.clone());
    }
    Ok(data)
}

pub fn message_list(messages: &[Message]) -> Result<Json, AzureChatError> {
    let value = serde_json::to_value(messages)
        .map_err(|error| AzureChatError::azure(500, error.to_string()))?;
    serde_json::from_value(value).map_err(|error| AzureChatError::azure(500, error.to_string()))
}
