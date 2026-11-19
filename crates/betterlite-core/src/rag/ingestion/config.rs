use std::collections::BTreeMap;

use betterlite_config::rag::document::ingest::{RAGIngestEmbeddingOptions, RAGIngestOCROptions};
use serde::Deserialize;
use serde_json::value::RawValue;
use thiserror::Error;

pub const DEFAULT_CHUNKING_STRATEGY: &str = r#"{"type":"auto"}"#;

pub type VectorStoreConfig = BTreeMap<String, Box<RawValue>>;

pub type ChunkingStrategyMap = BTreeMap<String, Box<RawValue>>;

// raw JSON keeps every caller and credential key the providers read, unlike a typed struct
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngestionOptions {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub ocr: Option<RAGIngestOCROptions>,
    #[serde(default)]
    pub chunking_strategy: Option<ChunkingStrategyMap>,
    #[serde(default)]
    pub embedding: Option<RAGIngestEmbeddingOptions>,
    #[serde(default)]
    pub vector_store: Option<VectorStoreConfig>,
}

#[derive(Debug, Error)]
pub enum IngestionSetupError {
    #[error("`{field}` must be {expected}")]
    InvalidField {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the chunking strategy is not valid JSON")]
    ChunkingStrategy {
        #[source]
        source: serde_json::Error,
    },
    #[error("the stored credential value `{field}` is not valid JSON")]
    CredentialValue {
        field: String,
        #[source]
        source: serde_json::Error,
    },
}

pub fn chunking_strategy_value(
    requested: Option<ChunkingStrategyMap>,
) -> Result<Box<RawValue>, IngestionSetupError> {
    let Some(strategy) = requested.filter(|strategy| !strategy.is_empty()) else {
        return default_chunking_strategy();
    };
    serde_json::value::to_raw_value(&strategy)
        .map_err(|source| IngestionSetupError::ChunkingStrategy { source })
}

fn default_chunking_strategy() -> Result<Box<RawValue>, IngestionSetupError> {
    serde_json::from_str(DEFAULT_CHUNKING_STRATEGY)
        .map_err(|source| IngestionSetupError::ChunkingStrategy { source })
}
