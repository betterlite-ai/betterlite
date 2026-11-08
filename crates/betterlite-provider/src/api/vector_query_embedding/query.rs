use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingVector};

use crate::api::vector_store_contracts::config::{
    Params, PreparedRequest, SearchQuery, SearchRequest, VectorStoreConfig,
    VectorStoreEmbeddingExecutor,
};

use super::error::QueryEmbeddingError;

// LiteLLM's query-embedding search transform carries the embedding executor beside the base arguments
pub struct QuerySearchRequest<'request> {
    pub search: SearchRequest<'request>,
    pub embedding_executor: Option<&'request dyn VectorStoreEmbeddingExecutor>,
}

#[async_trait]
pub trait QueryEmbeddingConfig: VectorStoreConfig {
    fn transform_query_search_request(
        &self,
        request: QuerySearchRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error>;

    // LiteLLM's async default hands every argument, the executor included, to the sync transform once
    async fn transform_query_search_async(
        &self,
        request: QuerySearchRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        self.transform_query_search_request(request)
    }
}

// a text query stays as it is, and a sequence joins with one space without trimming an element
#[must_use]
pub fn query_text(query: &SearchQuery) -> String {
    match query {
        SearchQuery::Text(text) => text.clone(),
        SearchQuery::Sequence(parts) => parts.join(" "),
    }
}

// litellm_embedding_model is the only source of the model, and only a nonempty string is a model
pub fn query_embedding_model(litellm_params: &Params) -> Result<&str, QueryEmbeddingError> {
    match litellm_params.get("litellm_embedding_model") {
        Some(Json::Text(model)) if !model.is_empty() => Ok(model),
        _ => Err(QueryEmbeddingError::MissingModel),
    }
}

// only a mapping is a configuration; a JSON object already holds its keys as text
pub fn query_embedding_configuration(
    litellm_params: &Params,
) -> Result<Params, QueryEmbeddingError> {
    let Some(Json::Object(configuration)) = litellm_params.get("litellm_embedding_config") else {
        return Ok(Params::new());
    };
    let entries = configuration
        .iter()
        .map(|(key, value)| (Json::Text(key.clone()), value.clone()))
        .collect();
    embedding_configuration(entries)
}

// the mapping a Python dict can hold, whose non-text key becomes the text str(key) writes
pub fn embedding_configuration(entries: Vec<(Json, Json)>) -> Result<Params, QueryEmbeddingError> {
    let mut configuration = Params::new();
    for (key, value) in entries {
        let name = match key {
            Json::Text(name) => name,
            Json::Number(number) if number.is_i64() || number.is_u64() => number.to_string(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                return Err(QueryEmbeddingError::ConfigKey);
            }
        };
        configuration.insert(name, value);
    }
    Ok(configuration)
}

// LiteLLM validates only the first record's vector with TypeAdapter(list[float]), after the call
pub fn first_embedding_vector(
    records: &[EmbeddingRecord],
) -> Result<Vec<f64>, QueryEmbeddingError> {
    let first = records.first().ok_or(QueryEmbeddingError::NoFirstRecord)?;
    let EmbeddingVector::Floats(numbers) = &first.embedding else {
        return Err(QueryEmbeddingError::NotAVector);
    };
    numbers
        .iter()
        .map(|number| number.as_f64().ok_or(QueryEmbeddingError::NotAVector))
        .collect()
}
