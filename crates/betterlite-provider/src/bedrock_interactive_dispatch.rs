use std::error::Error as StdError;
use std::future::{Future, ready};
use std::pin::Pin;

pub mod error;
pub mod http;
pub mod passthrough;
pub mod realtime_bridge;
pub mod responses;
pub mod search;
pub mod vector_retrieve;

mod tests;

use error::BedrockInteractiveError;

// B010's arm carries no data because OperationRequest is built before an interactive call picks its leaf
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BedrockInteractiveRequest;

// The prepared arm stays a marker: the dispatcher answers with the leaf's own record
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BedrockInteractivePrepared;

// One executable operation per interactive leaf; the caller builds the future through the leaf module
pub enum BedrockInteractiveOperation<'call, Transport>
where
    Transport: StdError + 'static,
{
    Responses(responses::RunFuture<'call>),
    Passthrough(passthrough::RunFuture<'call>),
    Search(search::RunFuture<'call>),
    VectorRetrieve(vector_retrieve::RunFuture<'call>),
    RealtimeBridge(realtime_bridge::RunFuture<'call, Transport>),
    Unsupported { operation: &'static str },
}

#[derive(Debug)]
pub enum BedrockInteractiveResult {
    Responses(responses::Outcome),
    Passthrough(passthrough::Outcome),
    Search(search::Outcome),
    VectorRetrieve(vector_retrieve::Outcome),
    RealtimeBridge(realtime_bridge::Outcome),
}

pub type DispatchFuture<'call, Transport> = Pin<
    Box<
        dyn Future<Output = Result<BedrockInteractiveResult, BedrockInteractiveError<Transport>>>
            + Send
            + 'call,
    >,
>;

// The closed enum is the only entry: an operation this task does not own is reported, never guessed
#[must_use]
pub fn dispatch<Transport>(
    operation: BedrockInteractiveOperation<'_, Transport>,
) -> DispatchFuture<'_, Transport>
where
    Transport: StdError + Send + 'static,
{
    match operation {
        BedrockInteractiveOperation::Responses(future) => {
            Box::pin(async move { Ok(BedrockInteractiveResult::Responses(future.await?)) })
        }
        BedrockInteractiveOperation::Passthrough(future) => {
            Box::pin(async move { Ok(BedrockInteractiveResult::Passthrough(future.await?)) })
        }
        BedrockInteractiveOperation::Search(future) => {
            Box::pin(async move { Ok(BedrockInteractiveResult::Search(future.await?)) })
        }
        BedrockInteractiveOperation::VectorRetrieve(future) => {
            Box::pin(async move { Ok(BedrockInteractiveResult::VectorRetrieve(future.await?)) })
        }
        BedrockInteractiveOperation::RealtimeBridge(future) => {
            Box::pin(async move { Ok(BedrockInteractiveResult::RealtimeBridge(future.await?)) })
        }
        BedrockInteractiveOperation::Unsupported { operation } => {
            Box::pin(ready(Err(BedrockInteractiveError::Unsupported {
                operation,
            })))
        }
    }
}
