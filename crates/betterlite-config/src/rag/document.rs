use secrecy::SecretString;
use serde::{Deserialize, Serialize};

// secrecy::SecretString has no Serialize impl by design, so structs holding one derive Deserialize only.

const fn default_chunk_size() -> i64 {
    1000
}

const fn default_chunk_overlap() -> i64 {
    200
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RAGChunkingStrategy {
    #[serde(default = "default_chunk_size")]
    pub chunk_size: i64,
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separators: Option<Vec<String>>,
}

impl Default for RAGChunkingStrategy {
    fn default() -> Self {
        Self {
            chunk_size: default_chunk_size(),
            chunk_overlap: default_chunk_overlap(),
            separators: None,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_store_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_days: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_credential_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BedrockVectorStoreOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_store_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_bucket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_for_ingestion: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingestion_timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_credential_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_access_key_id: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_secret_access_key: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_session_token: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_region_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_role_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_session_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_profile_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_web_identity_token: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_sts_endpoint: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_external_id: Option<SecretString>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VertexAIVectorStoreOptions {
    pub vector_store_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex_project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex_location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex_credentials: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gcs_bucket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_for_import: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimension: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_metric: Option<S3VectorsDistanceMetric>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_filterable_metadata_keys: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_credential_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_access_key_id: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_secret_access_key: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_session_token: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_region_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_role_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_session_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_profile_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_web_identity_token: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_sts_endpoint: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr: Option<RAGIngestOCROptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunking_strategy: Option<RAGChunkingStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<RAGIngestEmbeddingOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    const fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::Openai => ("openai", "OpenAIRAGIngestion"),
            Self::Bedrock => ("bedrock", "BedrockRAGIngestion"),
            Self::Gemini => ("gemini", "GeminiRAGIngestion"),
            Self::S3Vectors => ("s3_vectors", "S3VectorsRAGIngestion"),
            Self::VertexAi => ("vertex_ai", "VertexAIRAGIngestion"),
        }
    }
}

pub trait ProviderIdentity: Sized {
    fn wire_name(self) -> &'static str;
    fn ingestion_class_name(self) -> &'static str;
    fn from_wire_name(name: &str) -> Option<Self>;
}

impl ProviderIdentity for IngestionProvider {
    fn wire_name(self) -> &'static str {
        self.names().0
    }

    fn ingestion_class_name(self) -> &'static str {
        self.names().1
    }

    fn from_wire_name(name: &str) -> Option<Self> {
        INGESTION_PROVIDERS
            .into_iter()
            .find(|&provider| provider.wire_name() == name)
    }
}
