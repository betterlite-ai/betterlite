use betterlite_models::identity::provider_id::ProviderId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorStoreConfigTag {
    Bedrock,
}

#[must_use]
pub fn select_vector_store_config(provider: ProviderId) -> Option<VectorStoreConfigTag> {
    if provider != ProviderId::Bedrock {
        return None;
    }
    Some(VectorStoreConfigTag::Bedrock)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorStoresConfigTag {
    OpenAI,
    AzureOpenAI,
    Vertex,
    VertexSearchAPI,
    Bedrock,
    PGVector,
    AzureAI,
    Milvus,
    Gemini,
    RAGFlow,
    S3Vectors,
    Valkey,
    MongoDB,
}

const DIRECT_TAGS: &[(ProviderId, VectorStoresConfigTag)] = &[
    (ProviderId::Openai, VectorStoresConfigTag::OpenAI),
    (ProviderId::Azure, VectorStoresConfigTag::AzureOpenAI),
    (ProviderId::Bedrock, VectorStoresConfigTag::Bedrock),
    (ProviderId::PgVector, VectorStoresConfigTag::PGVector),
    (ProviderId::AzureAi, VectorStoresConfigTag::AzureAI),
    (ProviderId::Milvus, VectorStoresConfigTag::Milvus),
    (ProviderId::Gemini, VectorStoresConfigTag::Gemini),
    (ProviderId::Ragflow, VectorStoresConfigTag::RAGFlow),
    (ProviderId::S3Vectors, VectorStoresConfigTag::S3Vectors),
    (ProviderId::Valkey, VectorStoresConfigTag::Valkey),
    (ProviderId::Mongodb, VectorStoresConfigTag::MongoDB),
];

#[must_use]
pub fn select_vector_stores_config(
    provider: ProviderId,
    api_type: Option<&str>,
) -> Option<VectorStoresConfigTag> {
    if provider == ProviderId::VertexAi {
        let tag = if api_type == Some("search_api") {
            VectorStoresConfigTag::VertexSearchAPI
        } else if api_type.is_none() || api_type == Some("rag_api") {
            VectorStoresConfigTag::Vertex
        } else {
            return None;
        };
        return Some(tag);
    }
    DIRECT_TAGS
        .iter()
        .find_map(|(row, tag)| (*row == provider).then_some(*tag))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorStoreFilesConfigTag {
    OpenAI,
}

#[must_use]
pub fn select_store_files_config(provider: ProviderId) -> Option<VectorStoreFilesConfigTag> {
    if provider != ProviderId::Openai {
        return None;
    }
    Some(VectorStoreFilesConfigTag::OpenAI)
}
