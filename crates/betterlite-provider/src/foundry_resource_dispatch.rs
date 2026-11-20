use std::future::{Future, ready};
use std::pin::Pin;

use thiserror::Error;

pub mod image_edit;
pub mod image_generation;
pub mod messages_count;
pub mod passthrough;
pub mod rerank;
pub mod responses_decisions;
pub mod vector_search;

mod tests;

// B010's arm carries no data because OperationRequest is built before a resource call picks its leaf
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FoundryResourceRequest;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FoundryResourcePrepared;

type DispatchFuture<'call> = Pin<
    Box<dyn Future<Output = Result<FoundryResourceResult, FoundryResourceError>> + Send + 'call>,
>;

// One executable operation per leaf call; the token count leaf runs a Send future of its own
pub enum FoundryResourceOperation<'call> {
    ImageGeneration(Box<image_generation::Dispatch<'call>>),
    ImageEdit(Box<image_edit::Dispatch<'call>>),
    Rerank(Box<rerank::Dispatch<'call>>),
    ResponsesDecisions(Box<responses_decisions::Dispatch<'call>>),
    VectorSearch(Box<vector_search::Dispatch<'call>>),
    Passthrough(Box<passthrough::Dispatch<'call>>),
    MessagesCount(messages_count::RunFuture<'call>),
    Unsupported { operation: &'static str },
}

pub enum FoundryResourceResult {
    ImageGeneration(image_generation::Outcome),
    ImageEdit(image_edit::Outcome),
    Rerank(rerank::Outcome),
    ResponsesDecisions(responses_decisions::Outcome),
    VectorSearch(vector_search::Outcome),
    Passthrough(passthrough::Outcome),
    MessagesCount(messages_count::Outcome),
}

#[derive(Debug, Error)]
pub enum FoundryResourceError {
    #[error(transparent)]
    ImageGeneration(#[from] image_generation::Error),
    #[error(transparent)]
    ImageEdit(#[from] image_edit::Error),
    #[error(transparent)]
    Rerank(#[from] rerank::Error),
    #[error(transparent)]
    ResponsesDecisions(#[from] responses_decisions::Error),
    #[error(transparent)]
    VectorSearch(#[from] vector_search::Error),
    #[error(transparent)]
    Passthrough(#[from] passthrough::Error),
    #[error(transparent)]
    MessagesCount(#[from] messages_count::Error),
    #[error(
        "betterlite does not run the {operation} operation through the foundry resource provider"
    )]
    Unsupported { operation: &'static str },
}

// The six sync leaves run before the future, so it carries only Send values
#[must_use]
pub fn dispatch(resource: FoundryResourceOperation<'_>) -> DispatchFuture<'_> {
    match resource {
        FoundryResourceOperation::ImageGeneration(leaf) => Box::pin(ready(
            leaf.run()
                .map(FoundryResourceResult::ImageGeneration)
                .map_err(FoundryResourceError::ImageGeneration),
        )),
        FoundryResourceOperation::ImageEdit(leaf) => Box::pin(ready(
            leaf.run()
                .map(FoundryResourceResult::ImageEdit)
                .map_err(FoundryResourceError::ImageEdit),
        )),
        FoundryResourceOperation::Rerank(leaf) => Box::pin(ready(
            leaf.run()
                .map(FoundryResourceResult::Rerank)
                .map_err(FoundryResourceError::Rerank),
        )),
        FoundryResourceOperation::ResponsesDecisions(leaf) => Box::pin(ready(
            leaf.run()
                .map(FoundryResourceResult::ResponsesDecisions)
                .map_err(FoundryResourceError::ResponsesDecisions),
        )),
        FoundryResourceOperation::VectorSearch(leaf) => Box::pin(ready(
            leaf.run()
                .map(FoundryResourceResult::VectorSearch)
                .map_err(FoundryResourceError::VectorSearch),
        )),
        FoundryResourceOperation::Passthrough(leaf) => Box::pin(ready(
            leaf.run()
                .map(FoundryResourceResult::Passthrough)
                .map_err(FoundryResourceError::Passthrough),
        )),
        FoundryResourceOperation::MessagesCount(future) => {
            Box::pin(async move { Ok(FoundryResourceResult::MessagesCount(future.await?)) })
        }
        FoundryResourceOperation::Unsupported { operation } => {
            Box::pin(ready(Err(FoundryResourceError::Unsupported { operation })))
        }
    }
}
