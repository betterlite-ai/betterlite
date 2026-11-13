use betterlite_models::decisions::Extra;
use serde::{Deserialize, Serialize};
use serde_json::Number;

pub const EMBEDDING_CACHE_FORMAT_VERSION: i64 = 2;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum EmbeddingPayload {
    Vector(Vec<Number>),
    Text(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CachedEmbedding {
    pub embedding: Option<EmbeddingPayload>,
    pub index: Option<i64>,
    pub object: Option<String>,
    pub model: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub prompt_tokens_details: Option<Extra>,
    pub format_version: i64,
}
