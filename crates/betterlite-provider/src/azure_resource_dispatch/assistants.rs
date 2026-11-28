use std::future::Future;
use std::pin::Pin;

use betterlite_azure::assistants::error::AssistantsRunError;
use betterlite_azure::assistants::operations::{AssistantsRunsApi, RunCall, RunInvocation};
use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_azure::azure_assistant_definitions::error::DefinitionsError;
use betterlite_azure::azure_assistant_definitions::operations::{
    AssistantsApi, AssistantsReply, ClientRequest, DefinitionCall,
};
use betterlite_azure::azure_message_attachments::error::MessagesError;
use betterlite_azure::azure_message_attachments::operations::{
    MessageCall, MessagesApi, MessagesReply,
};
use betterlite_azure::azure_thread_messages::error::ThreadMessagesError;
use betterlite_azure::azure_thread_messages::operations::{
    ThreadMessagesApi, ThreadOperation, ThreadReply,
};
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::records::RunReply;
use betterlite_openai::thread_runs::stream::{RunEventStream, RunStreamTransport};

pub type DefinitionsOutcome = AssistantsReply;
pub type ThreadsOutcome = ThreadReply;
pub type MessagesOutcome = MessagesReply;

pub type DefinitionsFuture<'call> =
    Pin<Box<dyn Future<Output = Result<DefinitionsOutcome, DefinitionsError>> + Send + 'call>>;
pub type ThreadsFuture<'call> =
    Pin<Box<dyn Future<Output = Result<ThreadsOutcome, ThreadMessagesError>> + Send + 'call>>;
pub type MessagesFuture<'call> =
    Pin<Box<dyn Future<Output = Result<MessagesOutcome, MessagesError>> + Send + 'call>>;
pub type RunsFuture<'call> =
    Pin<Box<dyn Future<Output = Result<RunOutcome, AssistantsRunError>> + Send + 'call>>;

// The SDK stream is generic in its transport, so the dispatch keeps the open step behind a boxed future
pub type RunEvents = Pin<Box<dyn Future<Output = Result<RunEventStream, ThreadRunsError>> + Send>>;

pub enum RunOutcome {
    Polled(ClientKind, Box<RunReply>),
    Streaming(ClientKind, RunEvents),
}

pub struct DefinitionsDispatch<'call, T: AssistantsTransport> {
    pub api: &'call AssistantsApi<T>,
    pub request: &'call ClientRequest<'call>,
    pub supplied: Option<AzureAssistantsClient<'call, T>>,
    pub flag: Option<bool>,
    pub call: DefinitionCall<'call>,
}

pub struct ThreadsDispatch<'call, T: AssistantsTransport> {
    pub api: &'call ThreadMessagesApi<T>,
    pub request: &'call ClientRequest<'call>,
    pub supplied: Option<AzureAssistantsClient<'call, T>>,
    pub flag: Option<bool>,
    pub call: ThreadOperation<'call>,
}

pub struct MessagesDispatch<'call, T: AssistantsTransport> {
    pub api: &'call MessagesApi<T>,
    pub request: &'call ClientRequest<'call>,
    pub supplied: Option<AzureAssistantsClient<'call, T>>,
    pub flag: Option<bool>,
    pub call: MessageCall<'call>,
}

pub struct RunsDispatch<'call, T: AssistantsTransport + RunStreamTransport> {
    pub api: &'call AssistantsRunsApi<T>,
    pub request: &'call ClientRequest<'call>,
    pub supplied: Option<AzureAssistantsClient<'call, T>>,
    pub call: RunCall<'call>,
}

impl<T: AssistantsTransport> DefinitionsDispatch<'_, T> {
    // B686's definition facades: the flag picks the client kind and the call picks the SDK method
    pub async fn run(self) -> Result<DefinitionsOutcome, DefinitionsError> {
        self.api
            .call(self.request, self.supplied, self.flag, self.call)
            .await
    }
}

impl<T: AssistantsTransport> ThreadsDispatch<'_, T> {
    // B687's thread facades, served by the same supplied or constructed client
    pub async fn run(self) -> Result<ThreadsOutcome, ThreadMessagesError> {
        self.api
            .call(self.request, self.supplied, self.flag, self.call)
            .await
    }
}

impl<T: AssistantsTransport> MessagesDispatch<'_, T> {
    // B697's message facades, including the attachment policy its append call applies
    pub async fn run(self) -> Result<MessagesOutcome, MessagesError> {
        self.api
            .call(self.request, self.supplied, self.flag, self.call)
            .await
    }
}

impl<T: AssistantsTransport + RunStreamTransport + 'static> RunsDispatch<'_, T> {
    // B543's run facade; a streaming run keeps the SDK's lazy open behind the returned future
    pub async fn run(self) -> Result<RunOutcome, AssistantsRunError> {
        match self.api.run(self.request, self.supplied, self.call).await? {
            RunInvocation::Polled(kind, reply) => Ok(RunOutcome::Polled(kind, reply)),
            RunInvocation::Streaming(stream) => {
                let kind = stream.kind;
                Ok(RunOutcome::Streaming(kind, Box::pin(stream.events())))
            }
        }
    }
}
