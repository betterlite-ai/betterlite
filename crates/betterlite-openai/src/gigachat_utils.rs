use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::openai_base_config::Object;

pub const GIGACHAT_BASE_URL: &str = "https://gigachat.devices.sberbank.ru/api/v1";

pub const TOOL_CALL_ID_PREFIX: &str = "call_";
pub const TOOL_CALL_ID_CHARS: usize = 24;
pub const CHAT_COMPLETION_ID_CHARS: usize = 12;

pub trait IdSource: Send + Sync {
    fn new_uuid(&self) -> Uuid;
}

pub(crate) fn generated_id(ids: &dyn IdSource, prefix: &str, chars: usize) -> String {
    let hex: String = ids
        .new_uuid()
        .simple()
        .to_string()
        .chars()
        .take(chars)
        .collect();
    format!("{prefix}{hex}")
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
    api_base
        .or_else(|| env_base.filter(|base| !base.is_empty()))
        .unwrap_or(GIGACHAT_BASE_URL)
        .to_owned()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct TokenUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub precached_prompt_tokens: Option<u64>,
    #[serde(flatten)]
    pub other: Object,
}

impl TokenUsage {
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

#[must_use]
pub fn convert_usage(usage: &TokenUsage) -> ConvertedUsage {
    let precached = usage.precached_prompt_tokens.unwrap_or_default();
    ConvertedUsage {
        prompt_tokens: usage
            .prompt_tokens
            .unwrap_or_default()
            .saturating_add(precached),
        completion_tokens: usage.completion_tokens.unwrap_or_default(),
        total_tokens: usage
            .total_tokens
            .unwrap_or_default()
            .saturating_add(precached),
        cached_tokens: (precached > 0).then_some(precached),
    }
}

#[must_use]
pub(crate) fn arguments_text(arguments: Option<&Value>) -> String {
    match arguments {
        None | Some(Value::Null) => String::from("{}"),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

mod tests;
