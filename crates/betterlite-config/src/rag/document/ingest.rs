use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::RAGChunkingStrategy;

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
    #[serde(rename = "custom_llm_provider")]
    _custom_llm_provider: Option<String>,
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
    #[serde(rename = "custom_llm_provider")]
    _custom_llm_provider: Option<String>,
    #[serde(default)]
    pub vector_store_id: Option<String>,
    #[serde(default)]
    pub knowledge_base_id: Option<String>,
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
    #[serde(default)]
    #[serde(rename = "custom_llm_provider")]
    _custom_llm_provider: Option<String>,
    #[serde(default)]
    pub vector_store_id: Option<String>,
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
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub vertex_ai_project: Option<String>,
    #[serde(default)]
    pub vertex_ai_location: Option<String>,
    #[serde(default)]
    pub vertex_ai_credentials: Option<SecretString>,
    #[serde(default)]
    pub file_description: Option<String>,
    #[serde(default)]
    pub max_embedding_requests_per_min: Option<i64>,
    #[serde(default)]
    pub litellm_credential_name: Option<String>,
    #[serde(default)]
    #[expect(
        clippy::disallowed_types,
        reason = "Vertex passes provider-specific vector database config through unchanged"
    )]
    pub vector_db_config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum SslVerify {
    Bool(bool),
    CaBundle(String),
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
    #[serde(default)]
    #[serde(rename = "custom_llm_provider")]
    _custom_llm_provider: Option<String>,
    #[serde(default)]
    pub vector_bucket_name: Option<String>,
    #[serde(default)]
    pub vector_store_id: Option<String>,
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
    #[serde(default)]
    pub ssl_verify: Option<SslVerify>,
    #[serde(default)]
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub litellm_embedding_model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeminiVectorStoreOptions {
    #[serde(default)]
    #[serde(rename = "custom_llm_provider")]
    _custom_llm_provider: Option<String>,
    #[serde(default)]
    pub vector_store_id: Option<String>,
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub litellm_credential_name: Option<String>,
    #[serde(default)]
    #[expect(
        clippy::disallowed_types,
        reason = "Gemini metadata is a provider-owned passthrough"
    )]
    pub custom_metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "Box<serde_json::value::RawValue>")]
pub enum RAGIngestVectorStoreOptions {
    Openai(OpenAIVectorStoreOptions),
    Bedrock(BedrockVectorStoreOptions),
    VertexAi(VertexAIVectorStoreOptions),
    S3Vectors(S3VectorsVectorStoreOptions),
    Gemini(GeminiVectorStoreOptions),
}

#[derive(Deserialize)]
struct ProviderTag {
    #[serde(default)]
    custom_llm_provider: Option<String>,
}

impl TryFrom<Box<serde_json::value::RawValue>> for RAGIngestVectorStoreOptions {
    type Error = VectorStoreOptionsError;

    fn try_from(raw: Box<serde_json::value::RawValue>) -> Result<Self, Self::Error> {
        let tag: ProviderTag = serde_json::from_str(raw.get())?;
        let provider = tag.custom_llm_provider.as_deref().unwrap_or("openai");
        match provider {
            "openai" => Ok(Self::Openai(serde_json::from_str(raw.get())?)),
            "bedrock" => Ok(Self::Bedrock(serde_json::from_str(raw.get())?)),
            "vertex_ai" => Ok(Self::VertexAi(serde_json::from_str(raw.get())?)),
            "s3_vectors" => Ok(Self::S3Vectors(serde_json::from_str(raw.get())?)),
            "gemini" => Ok(Self::Gemini(serde_json::from_str(raw.get())?)),
            _ => Err(VectorStoreOptionsError::UnknownProvider(
                provider.to_owned(),
            )),
        }
    }
}

#[derive(Debug, Error)]
pub enum VectorStoreOptionsError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("unknown variant `{0}`")]
    UnknownProvider(String),
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
