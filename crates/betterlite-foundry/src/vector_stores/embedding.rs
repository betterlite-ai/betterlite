use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingResponse, EmbeddingVector};

use super::environment::Params;
use super::error::{AzureSearchError, EmbeddingCallError};
use super::search::SearchQuery;

// B1058's VectorStoreEmbeddingExecutor protocol reaches this leaf through injection, never through an upper crate
#[async_trait]
pub trait QueryEmbeddingExecutor: Send + Sync {
    fn embed(
        &self,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, EmbeddingCallError>;

    async fn aembed(
        &self,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, EmbeddingCallError>;
}

// a text query stays as it is, and a sequence joins with one space without trimming an element
#[must_use]
pub fn query_text(query: &SearchQuery) -> String {
    match query {
        SearchQuery::Text(text) => text.clone(),
        SearchQuery::Sequence(parts) => parts.join(" "),
    }
}

pub fn query_embedding_model(litellm_params: &Params) -> Result<&str, AzureSearchError> {
    let Some(Json::Text(model)) = litellm_params.get("litellm_embedding_model") else {
        return Err(AzureSearchError::MissingEmbeddingModel);
    };
    if model.is_empty() {
        return Err(AzureSearchError::MissingEmbeddingModel);
    }
    Ok(model)
}

#[must_use]
pub fn query_embedding_configuration(litellm_params: &Params) -> Params {
    let Some(Json::Object(configuration)) = litellm_params.get("litellm_embedding_config") else {
        return Params::new();
    };
    configuration
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

// only the first record's vector is validated, and it keeps its own element order
pub fn first_embedding_vector(records: &[EmbeddingRecord]) -> Result<Vec<f64>, AzureSearchError> {
    let first = records.first().ok_or(AzureSearchError::NoFirstRecord)?;
    let EmbeddingVector::Floats(numbers) = &first.embedding else {
        return Err(AzureSearchError::NotAVector);
    };
    numbers
        .iter()
        .map(|number| number.as_f64().ok_or(AzureSearchError::NotAVector))
        .collect()
}

pub fn embed_query(
    query_text: &str,
    litellm_params: &Params,
    executor: &dyn QueryEmbeddingExecutor,
) -> Result<Vec<f64>, AzureSearchError> {
    let model = query_embedding_model(litellm_params)?;
    let configuration = query_embedding_configuration(litellm_params);
    let response = executor
        .embed(model, query_text, &configuration)
        .map_err(|source| AzureSearchError::Embedding { source })?;
    first_embedding_vector(&response.data)
}

pub async fn aembed_query(
    input: &str,
    litellm_params: &Params,
    embedder: &dyn QueryEmbeddingExecutor,
) -> Result<Vec<f64>, AzureSearchError> {
    let response = embedder
        .aembed(
            query_embedding_model(litellm_params)?,
            input,
            &query_embedding_configuration(litellm_params),
        )
        .await
        .map_err(|source| AzureSearchError::Embedding { source })?;
    first_embedding_vector(&response.data)
}
