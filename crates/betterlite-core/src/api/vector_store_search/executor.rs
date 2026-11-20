use super::error::VectorStoreSearchError;
use crate::api::vector_search_execution::config::{Params, QueryEmbeddingExecutor};

// The private _direct_vector_store_embedding_executor argument: a typed executor, or the object a caller supplied that is not one
#[derive(Clone, Copy)]
pub enum DirectExecutor<'executor> {
    Executor(&'executor dyn QueryEmbeddingExecutor),
    // Any other object fails LiteLLM's isinstance check with its TypeError
    Invalid,
}

// B1059's query_embedding_executor over the request metadata: the supplied executor wins, else the router one, else direct
pub trait EmbeddingExecutorSource {
    fn select<'selection>(
        &'selection self,
        supplied: Option<&'selection dyn QueryEmbeddingExecutor>,
        request_kwargs: &Params,
    ) -> SelectedExecutor<'selection>;
}

pub enum SelectedExecutor<'selection> {
    Supplied(&'selection dyn QueryEmbeddingExecutor),
    Built(Box<dyn QueryEmbeddingExecutor + 'selection>),
}

impl SelectedExecutor<'_> {
    pub fn as_executor(&self) -> &dyn QueryEmbeddingExecutor {
        match self {
            Self::Supplied(executor) => *executor,
            Self::Built(executor) => executor.as_ref(),
        }
    }
}

// _direct_vector_store_embedding_executor: the value is type-checked before the registry, the mocks or any HTTP
pub fn direct_executor<'selection>(
    value: Option<DirectExecutor<'selection>>,
    source: &'selection dyn EmbeddingExecutorSource,
    request_kwargs: &Params,
) -> Result<SelectedExecutor<'selection>, VectorStoreSearchError> {
    match value {
        None => Ok(source.select(None, request_kwargs)),
        Some(DirectExecutor::Executor(executor)) => {
            Ok(source.select(Some(executor), request_kwargs))
        }
        Some(DirectExecutor::Invalid) => Err(VectorStoreSearchError::InvalidEmbeddingExecutor),
    }
}
