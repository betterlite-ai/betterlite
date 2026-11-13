pub mod chunking;
pub mod ingest;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RAGIngestResponse {
    pub id: String,
    pub status: ingest::RAGIngestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_store_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestionProvider {
    Openai,
    Bedrock,
    Gemini,
    S3Vectors,
    VertexAi,
}

pub const INGESTION_PROVIDERS: [IngestionProvider; 5] = [
    IngestionProvider::Openai,
    IngestionProvider::Bedrock,
    IngestionProvider::Gemini,
    IngestionProvider::S3Vectors,
    IngestionProvider::VertexAi,
];

impl IngestionProvider {
    // one row per provider, mirroring LiteLLM's single INGESTION_REGISTRY dict.
    #[must_use]
    pub const fn wire_name_and_class(self) -> (&'static str, &'static str) {
        match self {
            Self::Openai => ("openai", "OpenAIRAGIngestion"),
            Self::Bedrock => ("bedrock", "BedrockRAGIngestion"),
            Self::Gemini => ("gemini", "GeminiRAGIngestion"),
            Self::S3Vectors => ("s3_vectors", "S3VectorsRAGIngestion"),
            Self::VertexAi => ("vertex_ai", "VertexAIRAGIngestion"),
        }
    }

    #[must_use]
    pub fn from_wire_name(name: &str) -> Option<Self> {
        match name {
            "openai" => Some(Self::Openai),
            "bedrock" => Some(Self::Bedrock),
            "gemini" => Some(Self::Gemini),
            "s3_vectors" => Some(Self::S3Vectors),
            "vertex_ai" => Some(Self::VertexAi),
            _ => None,
        }
    }
}
