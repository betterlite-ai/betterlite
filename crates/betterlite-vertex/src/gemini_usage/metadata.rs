use betterlite_models::decisions::Json;
use serde::Deserialize;

// vertex_ai.py declares TEXT/AUDIO/IMAGE/VIDEO; the API also sends DOCUMENT, and LiteLLM uppercases any string
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Modality {
    #[default]
    Other,
    Text,
    Audio,
    Image,
    Video,
    Document,
}

impl Modality {
    #[must_use]
    pub fn from_wire(wire: &str) -> Self {
        match wire.to_ascii_uppercase().as_str() {
            "TEXT" => Self::Text,
            "AUDIO" => Self::Audio,
            "IMAGE" => Self::Image,
            "VIDEO" => Self::Video,
            "DOCUMENT" => Self::Document,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct TokenCountDetail {
    pub modality: String,
    #[serde(rename = "tokenCount")]
    pub camel_case_token_count: Option<Json>,
    pub token_count: Option<Json>,
}

impl TokenCountDetail {
    // LiteLLM reads tokenCount first and only falls back to token_count when the camel key is absent
    #[must_use]
    pub fn token_count(&self) -> Option<i64> {
        self.camel_case_token_count
            .as_ref()
            .or(self.token_count.as_ref())
            .and_then(integer)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageMetadata {
    pub prompt_token_count: i64,
    pub candidates_token_count: i64,
    pub total_token_count: i64,
    pub response_token_count: Option<i64>,
    pub cached_content_token_count: Option<i64>,
    pub tool_use_prompt_token_count: Option<i64>,
    pub thoughts_token_count: Option<i64>,
    pub prompt_tokens_details: Option<Vec<TokenCountDetail>>,
    pub cache_tokens_details: Option<Vec<TokenCountDetail>>,
    pub response_tokens_details: Option<Vec<TokenCountDetail>>,
    pub candidates_tokens_details: Option<Vec<TokenCountDetail>>,
}

fn integer(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}
