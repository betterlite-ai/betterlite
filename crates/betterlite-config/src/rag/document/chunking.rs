use serde::{Deserialize, Serialize};

pub const DEFAULT_CHUNK_SIZE: i64 = 1000;
pub const DEFAULT_CHUNK_OVERLAP: i64 = 200;

fn default_separators() -> Vec<String> {
    ["\n\n", "\n", " ", ""]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

// Option's own Deserialize impl already accepts both an absent key and an explicit null.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RAGChunkingStrategyWire {
    #[serde(default)]
    chunk_size: Option<i64>,
    #[serde(default)]
    chunk_overlap: Option<i64>,
    #[serde(default, rename = "type")]
    kind: Option<ChunkingType>,
    #[serde(default, rename = "static")]
    static_tokens: Option<StaticChunkingStrategy>,
    #[serde(default)]
    white_space_config: Option<WhiteSpaceChunkingConfig>,
    #[serde(default)]
    separators: Option<Vec<String>>,
}

impl From<RAGChunkingStrategyWire> for RAGChunkingStrategy {
    fn from(wire: RAGChunkingStrategyWire) -> Self {
        Self {
            chunk_size: wire.chunk_size,
            chunk_overlap: wire.chunk_overlap,
            kind: wire.kind,
            static_tokens: wire.static_tokens,
            white_space_config: wire.white_space_config,
            separators: wire.separators.unwrap_or_else(default_separators),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RAGChunkingStrategyWire")]
pub struct RAGChunkingStrategy {
    pub chunk_size: Option<i64>,
    pub chunk_overlap: Option<i64>,
    pub separators: Vec<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<ChunkingType>,
    #[serde(rename = "static", skip_serializing_if = "Option::is_none")]
    pub static_tokens: Option<StaticChunkingStrategy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub white_space_config: Option<WhiteSpaceChunkingConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkingType {
    Auto,
    Static,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticChunkingStrategy {
    #[serde(default)]
    pub max_chunk_size_tokens: Option<i64>,
    #[serde(default)]
    pub chunk_overlap_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhiteSpaceChunkingConfig {
    #[serde(default)]
    pub max_tokens_per_chunk: Option<i64>,
    #[serde(default)]
    pub max_overlap_tokens: Option<i64>,
}

impl Default for RAGChunkingStrategy {
    fn default() -> Self {
        Self {
            chunk_size: None,
            chunk_overlap: None,
            separators: default_separators(),
            kind: None,
            static_tokens: None,
            white_space_config: None,
        }
    }
}
