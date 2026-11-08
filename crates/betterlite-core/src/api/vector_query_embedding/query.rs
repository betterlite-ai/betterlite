use betterlite_models::decisions::Json;
use betterlite_openai::embedding::{EmbeddingResponse, EmbeddingVector};
use betterlite_provider::api::vector_query_embedding::error::QueryEmbeddingError;
use betterlite_provider::api::vector_query_embedding::query::{
    first_embedding_vector, query_embedding_configuration, query_embedding_model,
};
use betterlite_provider::api::vector_store_contracts::config::{
    EmbeddingEntry, Params, VectorStoreEmbeddingExecutor,
};
use indexmap::IndexMap;

use crate::api::vector_store_contracts::executor::DirectEmbeddingExecutor;
use crate::api::vector_store_contracts::router::{RouterEmbeddingExecutor, VectorStoreRouter};

// LiteLLM's precedence: an explicit executor, else a router one with the request metadata, else direct
pub enum SelectedExecutor<'executor, R> {
    Explicit(&'executor dyn VectorStoreEmbeddingExecutor),
    Router(RouterEmbeddingExecutor<R>),
    Direct(DirectEmbeddingExecutor),
}

pub fn query_embedding_executor<R>(
    embedding_executor: Option<&dyn VectorStoreEmbeddingExecutor>,
    router: Option<(R, Params)>,
    direct: DirectEmbeddingExecutor,
) -> SelectedExecutor<'_, R> {
    match (embedding_executor, router) {
        (Some(explicit), _) => SelectedExecutor::Explicit(explicit),
        (None, Some((handle, metadata))) => SelectedExecutor::Router(RouterEmbeddingExecutor {
            router: handle,
            metadata,
            direct,
        }),
        (None, None) => SelectedExecutor::Direct(direct),
    }
}

impl<R: VectorStoreRouter> SelectedExecutor<'_, R> {
    pub fn as_executor(&self) -> &dyn VectorStoreEmbeddingExecutor {
        match self {
            SelectedExecutor::Explicit(explicit) => *explicit,
            SelectedExecutor::Router(router) => router,
            SelectedExecutor::Direct(direct) => direct,
        }
    }
}

// LiteLLM's embed_query and aembed_query differ only in the executor entry point they call
pub async fn embed_query(
    entry: EmbeddingEntry,
    query_text: &str,
    litellm_params: &Params,
    embedding_executor: Option<&dyn VectorStoreEmbeddingExecutor>,
    direct: DirectEmbeddingExecutor,
) -> Result<Vec<f64>, QueryEmbeddingError> {
    let model = query_embedding_model(litellm_params)?;
    let configuration = query_embedding_configuration(litellm_params)?;
    // LiteLLM's query embedding methods pass no router, so an explicit executor wins and direct follows
    let selected = match embedding_executor {
        Some(explicit) => explicit,
        None => &direct,
    };
    let response = selected
        .embed(entry, model, query_text, &configuration)
        .await
        .map_err(|source| QueryEmbeddingError::Execution { source })?;
    first_embedding_vector(&embedding_records(&response))
}

// LiteLLM's data list is untyped, so the typed records go back to the shape its validation reads
fn embedding_records(response: &EmbeddingResponse) -> Vec<Json> {
    let mut records = Vec::with_capacity(response.data.len());
    for record in &response.data {
        let embedding = match &record.embedding {
            EmbeddingVector::Base64(text) => Json::Text(text.clone()),
            EmbeddingVector::Floats(numbers) => Json::List(
                numbers
                    .iter()
                    .map(|number| Json::Number(number.clone()))
                    .collect(),
            ),
        };
        records.push(Json::Object(IndexMap::from([(
            "embedding".to_owned(),
            embedding,
        )])));
    }
    records
}
