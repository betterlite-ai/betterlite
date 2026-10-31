use secrecy::SecretString;
use serde::{Deserialize, Serialize};

// secrecy::SecretString has no Serialize impl by design, so structs holding one derive Deserialize only.

const fn default_chunk_size() -> i64 {
    1000
}

const fn default_chunk_overlap() -> i64 {
    200
}

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
    #[serde(default = "default_chunk_size")]
    chunk_size: i64,
    #[serde(default = "default_chunk_overlap")]
    chunk_overlap: i64,
    #[serde(default)]
    separators: Option<Vec<String>>,
}

impl From<RAGChunkingStrategyWire> for RAGChunkingStrategy {
    fn from(wire: RAGChunkingStrategyWire) -> Self {
        Self {
            chunk_size: wire.chunk_size,
            chunk_overlap: wire.chunk_overlap,
            separators: wire.separators.unwrap_or_else(default_separators),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RAGChunkingStrategyWire")]
pub struct RAGChunkingStrategy {
    pub chunk_size: i64,
    pub chunk_overlap: i64,
    pub separators: Vec<String>,
}

impl Default for RAGChunkingStrategy {
    fn default() -> Self {
        Self {
            chunk_size: default_chunk_size(),
            chunk_overlap: default_chunk_overlap(),
            separators: default_separators(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RAGIngestOCROptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RAGIngestEmbeddingOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAIVectorStoreOptions {
    #[serde(default)]
    pub vector_store_id: Option<String>,
    #[serde(default)]
    pub ttl_days: Option<i64>,
    #[serde(default)]
    pub litellm_credential_name: Option<String>,
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub api_base: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BedrockVectorStoreOptions {
    #[serde(default)]
    pub vector_store_id: Option<String>,
    #[serde(default)]
    pub s3_bucket: Option<String>,
    #[serde(default)]
    pub s3_prefix: Option<String>,
    #[serde(default)]
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub data_source_id: Option<String>,
    #[serde(default)]
    pub wait_for_ingestion: Option<bool>,
    #[serde(default)]
    pub ingestion_timeout: Option<i64>,
    #[serde(default)]
    pub litellm_credential_name: Option<String>,
    #[serde(default)]
    pub aws_access_key_id: Option<SecretString>,
    #[serde(default)]
    pub aws_secret_access_key: Option<SecretString>,
    #[serde(default)]
    pub aws_session_token: Option<SecretString>,
    #[serde(default)]
    pub aws_region_name: Option<String>,
    #[serde(default)]
    pub aws_role_name: Option<String>,
    #[serde(default)]
    pub aws_session_name: Option<String>,
    #[serde(default)]
    pub aws_profile_name: Option<String>,
    #[serde(default)]
    pub aws_web_identity_token: Option<SecretString>,
    #[serde(default)]
    pub aws_sts_endpoint: Option<SecretString>,
    #[serde(default)]
    pub aws_external_id: Option<SecretString>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VertexAIVectorStoreOptions {
    pub vector_store_id: String,
    #[serde(default)]
    pub vertex_project: Option<String>,
    #[serde(default)]
    pub vertex_location: Option<String>,
    #[serde(default)]
    pub vertex_credentials: Option<SecretString>,
    #[serde(default)]
    pub gcs_bucket: Option<String>,
    #[serde(default)]
    pub wait_for_import: Option<bool>,
    #[serde(default)]
    pub import_timeout: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S3VectorsDistanceMetric {
    Cosine,
    Euclidean,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3VectorsVectorStoreOptions {
    pub vector_bucket_name: String,
    #[serde(default)]
    pub index_name: Option<String>,
    #[serde(default)]
    pub dimension: Option<i64>,
    #[serde(default)]
    pub distance_metric: Option<S3VectorsDistanceMetric>,
    #[serde(default)]
    pub non_filterable_metadata_keys: Option<Vec<String>>,
    #[serde(default)]
    pub litellm_credential_name: Option<String>,
    #[serde(default)]
    pub aws_access_key_id: Option<SecretString>,
    #[serde(default)]
    pub aws_secret_access_key: Option<SecretString>,
    #[serde(default)]
    pub aws_session_token: Option<SecretString>,
    #[serde(default)]
    pub aws_region_name: Option<String>,
    #[serde(default)]
    pub aws_role_name: Option<String>,
    #[serde(default)]
    pub aws_session_name: Option<String>,
    #[serde(default)]
    pub aws_profile_name: Option<String>,
    #[serde(default)]
    pub aws_web_identity_token: Option<SecretString>,
    #[serde(default)]
    pub aws_sts_endpoint: Option<SecretString>,
    #[serde(default)]
    pub aws_external_id: Option<SecretString>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "custom_llm_provider", rename_all = "snake_case")]
pub enum RAGIngestVectorStoreOptions {
    Openai(OpenAIVectorStoreOptions),
    Bedrock(BedrockVectorStoreOptions),
    VertexAi(VertexAIVectorStoreOptions),
    S3Vectors(S3VectorsVectorStoreOptions),
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RAGIngestOptions {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub ocr: Option<RAGIngestOCROptions>,
    #[serde(default)]
    pub chunking_strategy: Option<RAGChunkingStrategy>,
    #[serde(default)]
    pub embedding: Option<RAGIngestEmbeddingOptions>,
    #[serde(default)]
    pub vector_store: Option<RAGIngestVectorStoreOptions>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RAGIngestStatus {
    Completed,
    InProgress,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RAGIngestResponse {
    pub id: String,
    pub status: RAGIngestStatus,
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
