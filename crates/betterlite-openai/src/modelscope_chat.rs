use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use thiserror::Error;

use crate::chat::{ChatMessage, MessageContent};

mod tests;

pub const DEFAULT_BASE_URL: &str = "https://api-inference.modelscope.cn/v1";
pub const PROVIDER: &str = "modelscope";

#[derive(Debug, Error)]
pub enum ModelscopeChatError {
    #[error("the text of a content part in message {index} is not a string")]
    TextNotString { index: usize },
}

// the URL only ever sees the base it is given: the environment fallback happens in provider_info, not here
#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    let base = api_base
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_BASE_URL);
    if base.ends_with("/chat/completions") {
        return base.to_owned();
    }
    format!("{base}/chat/completions")
}

// LiteLLM chains the explicit base and key, then MODELSCOPE_API_BASE and MODELSCOPE_API_KEY, then the default base
#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    env_base: Option<&str>,
    api_key: Option<&SecretString>,
    env_key: Option<&SecretString>,
) -> (String, Option<SecretString>) {
    let base = api_base
        .filter(|value| !value.is_empty())
        .or_else(|| env_base.filter(|value| !value.is_empty()))
        .unwrap_or(DEFAULT_BASE_URL)
        .to_owned();
    let key = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .or_else(|| env_key.filter(|key| !key.expose_secret().is_empty()))
        .cloned();
    (base, key)
}

// LiteLLM turns a list whose every part is text into those texts joined with nothing, and leaves any other list alone
pub fn flatten_messages(messages: &mut [ChatMessage]) -> Result<(), ModelscopeChatError> {
    for (index, message) in messages.iter_mut().enumerate() {
        let Some(MessageContent::Parts(parts)) = &message.content else {
            continue;
        };
        if parts.iter().any(|part| part.kind != "text") {
            continue;
        }
        let mut text = String::new();
        for part in parts {
            match part.fields.get("text") {
                None | Some(Value::Null) => {}
                Some(Value::String(piece)) => text.push_str(piece),
                Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
                    return Err(ModelscopeChatError::TextNotString { index });
                }
            }
        }
        message.content = Some(MessageContent::Text(text));
    }
    Ok(())
}
