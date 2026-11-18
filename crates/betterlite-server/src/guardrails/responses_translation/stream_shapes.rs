#![expect(
    clippy::disallowed_types,
    reason = "Responses stream data is an open wire boundary"
)]

use serde::Deserialize;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ResponseOutputEnvelope {
    pub output: Option<Vec<serde_json::Value>>,
    pub model: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ResponsesStreamChunk {
    pub r#type: Option<String>,
    pub text: Option<String>,
    pub delta: Option<String>,
    pub item_id: Option<String>,
    pub output_index: Option<i64>,
    pub content_index: Option<i64>,
}

mod tests;
