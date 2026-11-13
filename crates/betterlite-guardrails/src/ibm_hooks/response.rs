use serde::{Deserialize, Serialize};

// The ModelResponse fields the post-call hook reads; the wire-format crate owns the full response.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ModelResponse {
    pub choices: Vec<ResponseChoice>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ResponseChoice {
    pub message: ResponseMessage,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ResponseMessage {
    pub content: Option<String>,
}
