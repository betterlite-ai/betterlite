use std::future::{Future, ready};
use std::pin::Pin;

use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::batches::error::BatchesError;
use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::fine_tuning::error::FineTuneError;
use betterlite_openai::vector_stores::error::VectorStoreError;
use thiserror::Error;

pub mod batches;
pub mod containers;
pub mod evals;
pub mod files;
pub mod fine_tuning;
pub mod vector_stores;

mod tests;

// B010's arm carries no data because OperationRequest is built before a resource call picks its leaf
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenAiResourceRequest;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenAiResourcePrepared;

// One executable operation per leaf call; the two generic leaves are boxed as the future they run
type FilesFuture<'call> =
    Pin<Box<dyn Future<Output = Result<files::Outcome, FilesError>> + Send + 'call>>;
type BatchesFuture<'call> =
    Pin<Box<dyn Future<Output = Result<batches::Outcome, BatchesError>> + Send + 'call>>;
type DispatchFuture<'call> =
    Pin<Box<dyn Future<Output = Result<OpenAiResourceResult, OpenAiResourceError>> + Send + 'call>>;

pub enum OpenAiResourceOperation<'call> {
    Files(FilesFuture<'call>),
    Batches(BatchesFuture<'call>),
    FineTuning(Box<fine_tuning::Dispatch<'call>>),
    Containers(Box<containers::Dispatch<'call>>),
    Evals(Box<evals::Dispatch<'call>>),
    VectorStores(Box<vector_stores::Dispatch<'call>>),
    Unsupported { operation: &'static str },
}

pub enum OpenAiResourceResult {
    Files(files::Outcome),
    Batches(batches::Outcome),
    FineTuning(fine_tuning::Outcome),
    Containers(containers::Outcome),
    Evals(evals::Outcome),
    VectorStores(vector_stores::Outcome),
}

#[derive(Debug, Error)]
pub enum OpenAiResourceError {
    #[error(transparent)]
    Files(#[from] FilesError),
    #[error(transparent)]
    Batches(#[from] BatchesError),
    #[error(transparent)]
    FineTuning(#[from] FineTuneError),
    #[error(transparent)]
    Containers(#[from] ContainerError),
    #[error(transparent)]
    Evals(#[from] EvalsError),
    #[error(transparent)]
    VectorStores(#[from] VectorStoreError),
    #[error(
        "betterlite does not run the {operation} operation through the openai resource provider"
    )]
    Unsupported { operation: &'static str },
}

// The four sync leaves run before the future, so it carries only Send values
#[must_use]
pub fn dispatch(resource: OpenAiResourceOperation<'_>) -> DispatchFuture<'_> {
    match resource {
        OpenAiResourceOperation::Files(future) => {
            Box::pin(async move { Ok(OpenAiResourceResult::Files(future.await?)) })
        }
        OpenAiResourceOperation::Batches(future) => {
            Box::pin(async move { Ok(OpenAiResourceResult::Batches(future.await?)) })
        }
        OpenAiResourceOperation::FineTuning(leaf) => Box::pin(ready(
            leaf.run()
                .map(OpenAiResourceResult::FineTuning)
                .map_err(OpenAiResourceError::FineTuning),
        )),
        OpenAiResourceOperation::Containers(leaf) => Box::pin(ready(
            leaf.run()
                .map(OpenAiResourceResult::Containers)
                .map_err(OpenAiResourceError::Containers),
        )),
        OpenAiResourceOperation::Evals(leaf) => Box::pin(ready(
            leaf.run()
                .map(OpenAiResourceResult::Evals)
                .map_err(OpenAiResourceError::Evals),
        )),
        OpenAiResourceOperation::VectorStores(leaf) => Box::pin(ready(
            leaf.run()
                .map(OpenAiResourceResult::VectorStores)
                .map_err(OpenAiResourceError::VectorStores),
        )),
        OpenAiResourceOperation::Unsupported { operation } => {
            Box::pin(ready(Err(OpenAiResourceError::Unsupported { operation })))
        }
    }
}
