use async_trait::async_trait;
use betterlite_core::api::vector_search_execution::config::{
    EmbeddingEntry, Params, QueryEmbeddingExecutor, StoreFailure,
};
use betterlite_core::api::vector_store_search::executor::{
    EmbeddingExecutorSource, SelectedExecutor,
};
use betterlite_openai::embedding::EmbeddingResponse;

// B1059 with no router: the built executor is never reached by the OpenAI config
pub(super) struct Executors;

impl EmbeddingExecutorSource for Executors {
    fn select<'selection>(
        &'selection self,
        supplied: Option<&'selection dyn QueryEmbeddingExecutor>,
        _request_kwargs: &Params,
    ) -> SelectedExecutor<'selection> {
        match supplied {
            Some(executor) => SelectedExecutor::Supplied(executor),
            None => SelectedExecutor::Built(Box::new(UnusedExecutor)),
        }
    }
}

struct UnusedExecutor;

#[async_trait]
impl QueryEmbeddingExecutor for UnusedExecutor {
    async fn embed(
        &self,
        _entry: EmbeddingEntry,
        _model: &str,
        _query: &str,
        _configuration: &Params,
    ) -> Result<EmbeddingResponse, StoreFailure> {
        Err(StoreFailure::from(
            "the oracle supplies no query embedding executor",
        ))
    }
}
