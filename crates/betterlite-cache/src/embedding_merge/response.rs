use betterlite_models::decisions::{Extra, HiddenParams};
use rust_decimal::Decimal;

use crate::types::embedding::EmbeddingPayload;

use super::usage::Usage;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Embedding {
    pub embedding: EmbeddingPayload,
    pub index: i64,
}

// A position holds None until a fresh embedding fills it
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CachedEmbeddingResponse {
    pub model: Option<String>,
    pub data: Vec<Option<Embedding>>,
    pub usage: Option<Usage>,
    pub hidden_params: HiddenParams,
    pub response_headers: Option<Extra>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApiEmbeddingResponse {
    pub data: Vec<Embedding>,
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergedEmbeddingResponse {
    pub model: Option<String>,
    pub data: Vec<Embedding>,
    pub usage: Usage,
    pub hidden_params: HiddenParams,
    pub response_headers: Option<Extra>,
    pub response_ms: Decimal,
}
