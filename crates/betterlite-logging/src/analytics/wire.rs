use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Usage {
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub prompt_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Tokens {
    pub completion: u64,
    pub prompt: u64,
}

impl From<&Usage> for Tokens {
    fn from(usage: &Usage) -> Self {
        Self {
            completion: usage.completion_tokens,
            prompt: usage.prompt_tokens,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Metadata<Fields> {
    pub parent_run_id: Option<String>,
    pub tags: Option<Vec<String>>,
    #[serde(flatten)]
    pub fields: Fields,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Request<Fields> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<ChatMessage>>,
    #[serde(flatten)]
    pub fields: Fields,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Choice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<ChatMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Object {
    Known(KnownObject),
    Unknown(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum KnownObject {
    #[serde(rename = "edit")]
    Edit,
    #[serde(rename = "text_completion")]
    Completion,
    #[serde(rename = "chat.completion")]
    Chat,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Response<Fields> {
    pub object: Object,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub created: Decimal,
    pub choices: Vec<Choice>,
    #[serde(flatten)]
    pub fields: Fields,
}
