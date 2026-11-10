use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::openai_base_config::Object;

pub const GIGACHAT_BASE_URL: &str = "https://gigachat.devices.sberbank.ru/api/v1";

// LiteLLM names a generated tool call and a generated chat completion the same way in the reply and the stream
pub const TOOL_CALL_ID_PREFIX: &str = "call_";
pub const TOOL_CALL_ID_CHARS: usize = 24;
pub const CHAT_COMPLETION_ID_CHARS: usize = 12;

// LiteLLM builds every generated id from uuid4, so the source is injected and tests fix it
pub trait IdSource: Send + Sync {
    fn new_uuid(&self) -> Uuid;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UuidIds;

impl IdSource for UuidIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::new_v4()
    }
}

#[must_use]
pub fn get_api_base(api_base: Option<&str>, env_base: Option<&str>) -> String {
    if let Some(base) = api_base {
        return base.to_owned();
    }
    match env_base {
        Some(base) if !base.is_empty() => base.to_owned(),
        Some(_) | None => GIGACHAT_BASE_URL.to_owned(),
    }
}

// GigaChat's usage block: LiteLLM reads each counter with 0 when the key is missing
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct TokenUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub precached_prompt_tokens: Option<u64>,
    // a key betterlite does not read still makes the block present, as any non-empty mapping does in LiteLLM
    #[serde(flatten)]
    pub other: Object,
}

impl TokenUsage {
    // LiteLLM skips an empty usage mapping, so an all-absent record is no usage at all
    #[must_use]
    pub fn is_present(&self) -> bool {
        !self.other.is_empty()
            || self.prompt_tokens.is_some()
            || self.completion_tokens.is_some()
            || self.total_tokens.is_some()
            || self.precached_prompt_tokens.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConvertedUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cached_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UsageError {
    #[error("the GigaChat token counters do not fit the total")]
    Overflow,
}

pub fn convert_usage(usage: &TokenUsage) -> Result<ConvertedUsage, UsageError> {
    let precached = usage.precached_prompt_tokens.unwrap_or_default();
    let prompt_tokens = usage
        .prompt_tokens
        .unwrap_or_default()
        .checked_add(precached)
        .ok_or(UsageError::Overflow)?;
    let total_tokens = usage
        .total_tokens
        .unwrap_or_default()
        .checked_add(precached)
        .ok_or(UsageError::Overflow)?;
    Ok(ConvertedUsage {
        prompt_tokens,
        completion_tokens: usage.completion_tokens.unwrap_or_default(),
        total_tokens,
        cached_tokens: (precached > 0).then_some(precached),
    })
}

// LiteLLM writes an object arguments as JSON and keeps any other value as its own text
#[must_use]
pub(crate) fn arguments_text(arguments: Option<&Value>) -> String {
    match arguments {
        None | Some(Value::Null) => String::from("{}"),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

mod tests;
