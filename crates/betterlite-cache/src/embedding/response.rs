use betterlite_models::decisions::Extra;
use serde::{Deserialize, Serialize};

use crate::types::embedding::EmbeddingPayload;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct EmbeddingResponse {
    pub model: Option<String>,
    #[serde(default)]
    pub data: Vec<EmbeddingRecord>,
    pub usage: Option<EmbeddingUsage>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct EmbeddingRecord {
    pub embedding: Option<EmbeddingPayload>,
    pub index: Option<i64>,
    pub object: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct EmbeddingUsage {
    pub prompt_tokens: Option<i64>,
    pub prompt_tokens_details: Option<Extra>,
}
