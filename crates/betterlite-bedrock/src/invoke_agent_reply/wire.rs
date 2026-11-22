use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

// The supplied buffered chat-completion record LiteLLM mutates: model may be None because the usage record always carries the key.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ModelResponse {
    pub id: String,
    pub created: i64,
    pub model: Option<String>,
    pub object: String,
    pub system_fingerprint: Option<String>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub usage: Option<Usage>,
    pub service_tier: Option<Json>,
    pub trace: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Choice {
    pub finish_reason: String,
    pub index: u32,
    pub message: Message,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Message {
    pub content: Option<String>,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}
