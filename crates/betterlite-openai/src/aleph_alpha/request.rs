use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use url::Url;

use crate::aleph_alpha::config::AlephAlphaConfig;
use crate::aleph_alpha::error::AlephAlphaError;
use crate::chat::{ChatMessage, MessageContent, Role};
use crate::openai_base_config::Object;

pub struct CompletionInput<'call> {
    pub model: &'call str,
    pub messages: &'call [ChatMessage],
    pub api_base: &'call str,
    pub api_key: Option<&'call SecretString>,
    pub optional_params: &'call Object,
    pub config: &'call AlephAlphaConfig,
    // the caller's model_response.created, resolved from the injected clock before the call
    pub created: i64,
}

#[derive(Debug)]
pub struct CompletionRequest {
    pub url: Url,
    pub prompt: String,
    pub headers: HeaderMap,
    pub body: Object,
    pub stream: bool,
}

pub fn completion_request(
    input: &CompletionInput<'_>,
) -> Result<CompletionRequest, AlephAlphaError> {
    let prompt = build_prompt(input.model, input.messages)?;
    let mut options = input.optional_params.clone();
    input.config.merge_defaults(&mut options);
    let stream = match options.get("stream") {
        None => false,
        Some(Value::Bool(given)) => *given,
        Some(_) => return Err(AlephAlphaError::StreamType),
    };
    let mut body = Object::from_iter([
        (String::from("model"), Value::String(input.model.to_owned())),
        (String::from("prompt"), Value::String(prompt.clone())),
    ]);
    body.extend(options);
    let url = Url::parse(input.api_base).map_err(|source| AlephAlphaError::Endpoint { source })?;
    Ok(CompletionRequest {
        url,
        prompt,
        headers: validate_environment(input.api_key)?,
        body,
        stream,
    })
}

// LiteLLM sends these two lowercase headers and adds a bearer token only for a non-empty key
pub fn validate_environment(api_key: Option<&SecretString>) -> Result<HeaderMap, AlephAlphaError> {
    let mut headers = HeaderMap::from_iter([
        (ACCEPT, HeaderValue::from_static("application/json")),
        (CONTENT_TYPE, HeaderValue::from_static("application/json")),
    ]);
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        let value =
            HeaderValue::try_from(format!("Bearer {}", key.expose_secret())).map_err(|source| {
                AlephAlphaError::Header {
                    name: "authorization",
                    source,
                }
            })?;
        headers.insert(AUTHORIZATION, value);
    }
    Ok(headers)
}

pub fn build_prompt(model: &str, messages: &[ChatMessage]) -> Result<String, AlephAlphaError> {
    let control = model.contains("control");
    let mut prompt = String::new();
    for (index, message) in messages.iter().enumerate() {
        if control {
            prompt.push_str(control_label(index, message.role));
        }
        if !control && index > 0 {
            prompt.push(' ');
        }
        prompt.push_str(message_text(index, message)?);
    }
    Ok(prompt)
}

// the first message is the instruction whatever its role, and only a later system message adds another
const fn control_label(index: usize, role: Role) -> &'static str {
    if index == 0 {
        return "###Instruction: ";
    }
    match role {
        Role::System => "###Instruction: ",
        Role::User => "###Input: ",
        Role::Developer | Role::Assistant | Role::Tool | Role::Function => "###Response: ",
    }
}

fn message_text(index: usize, message: &ChatMessage) -> Result<&str, AlephAlphaError> {
    match &message.content {
        Some(MessageContent::Text(text)) => Ok(text),
        None | Some(MessageContent::Parts(_)) => Err(AlephAlphaError::Content { index }),
    }
}
