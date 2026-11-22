use std::future::{Future, ready};
use std::pin::Pin;

use betterlite_azure::assistants::error::AssistantsRunError;
use betterlite_azure::azure_assistant_definitions::error::DefinitionsError;
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_message_attachments::error::MessagesError;
use betterlite_azure::azure_thread_messages::error::ThreadMessagesError;
use betterlite_azure::batches::error::AzureBatchesError;
use betterlite_azure::fine_tuning::error::AzureFineTuneError;
use betterlite_azure::passthrough::error::AzurePassthroughError;
use betterlite_azure::realtime::error::RealtimeError;
use betterlite_azure::resource_urls::error::ResourceUrlError;
use betterlite_azure::responses::error::ResponsesError;
use betterlite_azure::search::error::AzureSearchError;
use thiserror::Error;

pub mod assistants;
pub mod batches;
pub mod files;
pub mod fine_tuning;
pub mod passthrough;
pub mod realtime;
pub mod resource_urls;
pub mod responses;
pub mod search;

mod tests;

// B010's arm carries no data because OperationRequest is built before a resource call picks its leaf
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureResourceRequest;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureResourcePrepared;

type DispatchFuture<'call> =
    Pin<Box<dyn Future<Output = Result<AzureResourceResult, AzureResourceError>> + Send + 'call>>;

// One executable operation per completed leaf; the leaves with a generic client are already boxed as their future
pub enum AzureResourceOperation<'call> {
    Files(files::RunFuture<'call>),
    Batches(batches::RunFuture<'call>),
    FineTuning(fine_tuning::RunFuture<'call>),
    AssistantDefinitions(assistants::DefinitionsFuture<'call>),
    AssistantThreads(assistants::ThreadsFuture<'call>),
    AssistantMessages(assistants::MessagesFuture<'call>),
    AssistantRuns(assistants::RunsFuture<'call>),
    Responses(Box<responses::Dispatch<'call>>),
    Realtime(Box<realtime::Dispatch<'call>>),
    Search(Box<search::Dispatch<'call>>),
    ResourceUrls(Box<resource_urls::Dispatch<'call>>),
    Passthrough(Box<passthrough::Dispatch<'call>>),
    Unsupported { operation: &'static str },
}

pub enum AzureResourceResult {
    Files(files::Outcome),
    Batches(batches::Outcome),
    FineTuning(fine_tuning::Outcome),
    AssistantDefinitions(assistants::DefinitionsOutcome),
    AssistantThreads(assistants::ThreadsOutcome),
    AssistantMessages(assistants::MessagesOutcome),
    AssistantRuns(assistants::RunOutcome),
    Responses(responses::Outcome),
    Realtime(realtime::Outcome),
    Search(search::Outcome),
    ResourceUrls(resource_urls::Outcome),
    Passthrough(passthrough::Outcome),
}

#[derive(Debug, Error)]
pub enum AzureResourceError {
    #[error(transparent)]
    Files(#[from] AzureFilesError),
    #[error(transparent)]
    Batches(#[from] AzureBatchesError),
    #[error(transparent)]
    FineTuning(#[from] AzureFineTuneError),
    #[error(transparent)]
    AssistantDefinitions(#[from] DefinitionsError),
    #[error(transparent)]
    AssistantThreads(#[from] ThreadMessagesError),
    #[error(transparent)]
    AssistantMessages(#[from] MessagesError),
    #[error(transparent)]
    AssistantRuns(#[from] AssistantsRunError),
    #[error(transparent)]
    Responses(#[from] ResponsesError),
    #[error(transparent)]
    Realtime(#[from] RealtimeError),
    #[error(transparent)]
    Search(#[from] AzureSearchError),
    #[error(transparent)]
    ResourceUrls(#[from] ResourceUrlError),
    #[error(transparent)]
    Passthrough(#[from] AzurePassthroughError),
    #[error(
        "betterlite does not run the {operation} operation through the azure resource provider"
    )]
    Unsupported { operation: &'static str },
}

// The five sync leaves run before the future, so it carries only Send values
#[must_use]
pub fn dispatch(resource: AzureResourceOperation<'_>) -> DispatchFuture<'_> {
    match resource {
        AzureResourceOperation::Files(future) => map_future(future, AzureResourceResult::Files),
        AzureResourceOperation::Batches(future) => map_future(future, AzureResourceResult::Batches),
        AzureResourceOperation::FineTuning(future) => {
            map_future(future, AzureResourceResult::FineTuning)
        }
        AzureResourceOperation::AssistantDefinitions(future) => {
            map_future(future, AzureResourceResult::AssistantDefinitions)
        }
        AzureResourceOperation::AssistantThreads(future) => {
            map_future(future, AzureResourceResult::AssistantThreads)
        }
        AzureResourceOperation::AssistantMessages(future) => {
            map_future(future, AzureResourceResult::AssistantMessages)
        }
        AzureResourceOperation::AssistantRuns(future) => {
            map_future(future, AzureResourceResult::AssistantRuns)
        }
        AzureResourceOperation::Responses(leaf) => {
            map_ready(leaf.run(), AzureResourceResult::Responses)
        }
        AzureResourceOperation::Realtime(leaf) => {
            map_ready(leaf.run(), AzureResourceResult::Realtime)
        }
        AzureResourceOperation::Search(leaf) => map_ready(leaf.run(), AzureResourceResult::Search),
        AzureResourceOperation::ResourceUrls(leaf) => {
            map_ready(leaf.run(), AzureResourceResult::ResourceUrls)
        }
        AzureResourceOperation::Passthrough(leaf) => {
            map_ready(leaf.run(), AzureResourceResult::Passthrough)
        }
        AzureResourceOperation::Unsupported { operation } => {
            Box::pin(ready(Err(AzureResourceError::Unsupported { operation })))
        }
    }
}

fn map_future<'call, F, T, E>(
    future: F,
    wrap: fn(T) -> AzureResourceResult,
) -> DispatchFuture<'call>
where
    F: Future<Output = Result<T, E>> + Send + 'call,
    AzureResourceError: From<E>,
{
    Box::pin(async move {
        let outcome = future.await?;
        Ok(wrap(outcome))
    })
}

fn map_ready<T, E>(
    result: Result<T, E>,
    wrap: fn(T) -> AzureResourceResult,
) -> DispatchFuture<'static>
where
    AzureResourceError: From<E>,
{
    match result {
        Ok(outcome) => Box::pin(ready(Ok(wrap(outcome)))),
        Err(error) => Box::pin(ready(Err(AzureResourceError::from(error)))),
    }
}
