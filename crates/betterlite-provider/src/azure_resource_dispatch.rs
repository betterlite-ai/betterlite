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
        AzureResourceOperation::Files(future) => {
            Box::pin(async move { Ok(AzureResourceResult::Files(future.await?)) })
        }
        AzureResourceOperation::Batches(future) => {
            Box::pin(async move { Ok(AzureResourceResult::Batches(future.await?)) })
        }
        AzureResourceOperation::FineTuning(future) => {
            Box::pin(async move { Ok(AzureResourceResult::FineTuning(future.await?)) })
        }
        AzureResourceOperation::AssistantDefinitions(future) => {
            Box::pin(async move { Ok(AzureResourceResult::AssistantDefinitions(future.await?)) })
        }
        AzureResourceOperation::AssistantThreads(future) => {
            Box::pin(async move { Ok(AzureResourceResult::AssistantThreads(future.await?)) })
        }
        AzureResourceOperation::AssistantMessages(future) => {
            Box::pin(async move { Ok(AzureResourceResult::AssistantMessages(future.await?)) })
        }
        AzureResourceOperation::AssistantRuns(future) => {
            Box::pin(async move { Ok(AzureResourceResult::AssistantRuns(future.await?)) })
        }
        AzureResourceOperation::Responses(leaf) => Box::pin(ready(
            leaf.run()
                .map(AzureResourceResult::Responses)
                .map_err(AzureResourceError::Responses),
        )),
        AzureResourceOperation::Realtime(leaf) => Box::pin(ready(
            leaf.run()
                .map(AzureResourceResult::Realtime)
                .map_err(AzureResourceError::Realtime),
        )),
        AzureResourceOperation::Search(leaf) => Box::pin(ready(
            leaf.run()
                .map(AzureResourceResult::Search)
                .map_err(AzureResourceError::Search),
        )),
        AzureResourceOperation::ResourceUrls(leaf) => Box::pin(ready(
            leaf.run()
                .map(AzureResourceResult::ResourceUrls)
                .map_err(AzureResourceError::ResourceUrls),
        )),
        AzureResourceOperation::Passthrough(leaf) => Box::pin(ready(
            leaf.run()
                .map(AzureResourceResult::Passthrough)
                .map_err(AzureResourceError::Passthrough),
        )),
        AzureResourceOperation::Unsupported { operation } => {
            Box::pin(ready(Err(AzureResourceError::Unsupported { operation })))
        }
    }
}
