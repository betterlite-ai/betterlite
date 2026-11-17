use betterlite_config::rag::document::chunking::{
    ChunkingType, DEFAULT_CHUNK_OVERLAP, DEFAULT_CHUNK_SIZE, StaticChunkingStrategy,
    WhiteSpaceChunkingConfig,
};
use serde::{Deserialize, Serialize};

pub const SEPARATORS_IGNORED_WARNING: &str = "Vertex AI RAG Engine does not support custom separators. The 'separators' parameter will be ignored.";

// LiteLLM's DEFAULT_CHUNK_SIZE and DEFAULT_CHUNK_OVERLAP, injected by the config loader
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkDefaults {
    pub chunk_size: i64,
    pub chunk_overlap: i64,
}

impl ChunkDefaults {
    pub const LITELLM: Self = Self {
        chunk_size: DEFAULT_CHUNK_SIZE,
        chunk_overlap: DEFAULT_CHUNK_OVERLAP,
    };
}

// LiteLLM's RAGChunkingStrategy TypedDict: separators stay optional so an unsent list is not a warning
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RagChunkingStrategy {
    #[serde(default)]
    pub chunk_size: Option<i64>,
    #[serde(default)]
    pub chunk_overlap: Option<i64>,
    #[serde(default)]
    pub separators: Option<Vec<String>>,
    #[serde(default, rename = "type")]
    pub kind: Option<ChunkingType>,
    #[serde(default, rename = "static")]
    pub static_tokens: Option<StaticChunkingStrategy>,
    #[serde(default)]
    pub white_space_config: Option<WhiteSpaceChunkingConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChunkingConfig {
    pub chunk_size: i64,
    pub chunk_overlap: i64,
}

// Vertex AI's TransformationConfig: separators reach neither field, since the corpus splits the file itself
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChunkingRecord {
    pub chunking_config: ChunkingConfig,
}

#[must_use]
pub fn transform_chunking_strategy(
    strategy: Option<&RagChunkingStrategy>,
    defaults: ChunkDefaults,
) -> ChunkingRecord {
    let Some(strategy) = strategy else {
        return ChunkingRecord {
            chunking_config: ChunkingConfig {
                chunk_size: defaults.chunk_size,
                chunk_overlap: defaults.chunk_overlap,
            },
        };
    };
    if strategy
        .separators
        .as_ref()
        .is_some_and(|separators| !separators.is_empty())
    {
        tracing::warn!("{SEPARATORS_IGNORED_WARNING}");
    }
    ChunkingRecord {
        chunking_config: ChunkingConfig {
            chunk_size: strategy.chunk_size.unwrap_or(defaults.chunk_size),
            chunk_overlap: strategy.chunk_overlap.unwrap_or(defaults.chunk_overlap),
        },
    }
}

mod tests;
