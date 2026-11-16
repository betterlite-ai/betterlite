use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Message, convert_content_list_to_str};
use secrecy::{ExposeSecret as _, SecretString};

use crate::ollama_chat::ids::IdSource;

use super::error::A2aFailure;
use super::records::{
    A2aConfiguration, A2aMessage, A2aMethod, A2aParams, A2aRequestBody, A2aTextPart,
};

pub const SUPPORTED_OPENAI_PARAMS: [&str; 4] = ["stream", "temperature", "max_tokens", "top_p"];

// LiteLLM copies stream only when it is exactly true, since the method and blocking flag follow it
pub fn map_openai_params(non_default: &Object, optional: &mut Object) {
    if let Some(Json::Bool(true)) = non_default.get("stream") {
        optional.insert(String::from("stream"), Json::Bool(true));
    }
}

#[must_use]
pub fn validate_environment(
    headers: &BTreeMap<String, String>,
    api_key: Option<&SecretString>,
) -> BTreeMap<String, String> {
    let mut validated = headers.clone();
    if !headers.contains_key("content-type") && !headers.contains_key("Content-Type") {
        validated.insert(
            String::from("Content-Type"),
            String::from("application/json"),
        );
    }
    if let Some(key) = api_key {
        validated.insert(
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    validated
}

pub fn get_complete_url(api_base: Option<&str>) -> Result<String, A2aFailure> {
    match api_base {
        Some(base) => Ok(base.trim_end_matches('/').to_owned()),
        None => Err(A2aFailure::MissingApiBase),
    }
}

#[must_use]
pub fn convert_messages_to_prompt(messages: &[Message]) -> String {
    let mut parts = Vec::new();
    for message in messages {
        let content = convert_content_list_to_str(message);
        if content.is_empty() {
            continue;
        }
        parts.push(format!("{}: {content}", message.role));
    }
    parts.join("\n")
}

pub(crate) fn stream_requested(options: &Object) -> Result<bool, A2aFailure> {
    match options.get("stream") {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(stream)) => Ok(*stream),
        Some(Json::Text(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(A2aFailure::OptionType { field: "stream" })
        }
    }
}

pub fn transform_request(
    messages: &[Message],
    optional_params: &Object,
    ids: &dyn IdSource,
) -> Result<A2aRequestBody, A2aFailure> {
    let request_id = ids.next_id();
    if messages.is_empty() {
        return Err(A2aFailure::MessagesRequired);
    }
    let text = convert_messages_to_prompt(messages);
    let stream = stream_requested(optional_params)?;
    let method = if stream {
        A2aMethod::Stream
    } else {
        A2aMethod::Send
    };
    Ok(A2aRequestBody {
        jsonrpc: "2.0",
        id: request_id,
        method,
        params: A2aParams {
            message: A2aMessage {
                kind: "message",
                role: "user",
                parts: vec![A2aTextPart { kind: "text", text }],
                message_id: ids.next_id(),
            },
            configuration: (!stream).then_some(A2aConfiguration { blocking: true }),
        },
    })
}
