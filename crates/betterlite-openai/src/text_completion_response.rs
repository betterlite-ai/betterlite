use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// B2238's normalization forces object and defaults usage and every choice field; extra keeps the provider's other fields
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CompletionResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub object: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<CompletionChoice>,
    #[serde(default)]
    pub usage: Option<CompletionUsage>,
    #[serde(default)]
    pub system_fingerprint: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// B188's redact_choice blanks the text of a choice when message logging is turned off, so this type keeps that shape
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CompletionChoice {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub index: i64,
    #[serde(default)]
    pub logprobs: Option<Value>,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CompletionUsage {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub completion_tokens_details: Option<Value>,
    #[serde(default)]
    pub prompt_tokens_details: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

mod tests;
