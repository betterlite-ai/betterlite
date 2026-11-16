use std::future::Future;
use std::pin::Pin;

use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::batches::error::BatchesError;
use betterlite_openai::batches::records::BatchPage;
use betterlite_openai::batches::sdk::{BatchesSdk, ClientConstructor};
use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::transport::FileTransport;
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
type FilesFuture<'a> =
    Pin<Box<dyn Future<Output = Result<files::Outcome, FilesError>> + Send + 'a>>;
type BatchesFuture<'a> =
    Pin<Box<dyn Future<Output = Result<batches::Outcome, BatchesError>> + Send + 'a>>;

pub enum OpenAiResourceOperation<'a> {
    Files(FilesFuture<'a>),
    Batches(BatchesFuture<'a>),
    FineTuning(Box<fine_tuning::Dispatch<'a>>),
    Containers(Box<containers::Dispatch<'a>>),
    Evals(Box<evals::Dispatch<'a>>),
    VectorStores(Box<vector_stores::Dispatch<'a>>),
    Unsupported { operation: &'static str },
}

impl<'a> OpenAiResourceOperation<'a> {
    #[must_use]
    pub fn files<T>(dispatch: files::Dispatch<'a, T>) -> Self
    where
        T: FileTransport + 'a,
    {
        Self::Files(Box::pin(dispatch.run()))
    }

    #[must_use]
    pub fn batches<C>(dispatch: batches::Dispatch<'a, C>) -> Self
    where
        C: ClientConstructor + 'a,
        C::Client: BatchesSdk<Page = BatchPage> + 'a,
    {
        Self::Batches(Box::pin(dispatch.run()))
    }
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

// The only dispatch a leaf call goes through; leaf errors are passed on unchanged, never rewritten
#[must_use]
pub async fn dispatch(
    operation: OpenAiResourceOperation<'_>,
) -> Result<OpenAiResourceResult, OpenAiResourceError> {
    match operation {
        OpenAiResourceOperation::Files(future) => Ok(OpenAiResourceResult::Files(future.await?)),
        OpenAiResourceOperation::Batches(future) => {
            Ok(OpenAiResourceResult::Batches(future.await?))
        }
        OpenAiResourceOperation::FineTuning(dispatch) => {
            Ok(OpenAiResourceResult::FineTuning(dispatch.run()?))
        }
        OpenAiResourceOperation::Containers(dispatch) => {
            Ok(OpenAiResourceResult::Containers(dispatch.run()?))
        }
        OpenAiResourceOperation::Evals(dispatch) => {
            Ok(OpenAiResourceResult::Evals(dispatch.run()?))
        }
        OpenAiResourceOperation::VectorStores(dispatch) => {
            Ok(OpenAiResourceResult::VectorStores(dispatch.run()?))
        }
        OpenAiResourceOperation::Unsupported { operation } => {
            Err(OpenAiResourceError::Unsupported { operation })
        }
    }
}
