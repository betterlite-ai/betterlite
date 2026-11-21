mod assistants;
mod messages;
mod run;
mod support;
mod threads;

use betterlite_azure::credentials::source::CredentialSources;
use betterlite_core::api::assistant_messages::service::Operation as MessageKind;
use betterlite_core::api::assistant_read_delete::request::AssistantOperation as ReadDeleteOperation;
use betterlite_core::api::assistant_threads::ThreadOperation;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::thread_runs::stream::{RunStream, RunStreamTransport};

use super::AssistantDispatcher;
use super::Merged;
use super::error::AssistantDispatchError;
use super::operation::AssistantOperation;
use super::outcome::AssistantOutcome;
use super::request::AssistantRequest;

// the facades' transport and credential bounds, named once instead of on every handler
pub(super) trait Deps: AssistantsTransport + RunStreamTransport {}
impl<T: AssistantsTransport + RunStreamTransport> Deps for T {}

pub(super) trait Sources: CredentialSources + Clone + Send + Sync {}
impl<S: CredentialSources + Clone + Send + Sync> Sources for S {}

pub(super) type Dispatched<T> = Result<AssistantOutcome<RunStream<T>>, AssistantDispatchError>;

pub(super) async fn run<T: Deps, S: Sources>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    request: &AssistantRequest<'_>,
    merged: Merged,
) -> Dispatched<T> {
    match request.operation {
        AssistantOperation::CreateAssistant => {
            assistants::create(dispatcher, merged, request).await
        }
        AssistantOperation::ListAssistants => {
            assistants::read_or_delete(dispatcher, &merged, ReadDeleteOperation::List, request)
                .await
        }
        AssistantOperation::DeleteAssistant => {
            assistants::read_or_delete(dispatcher, &merged, ReadDeleteOperation::Delete, request)
                .await
        }
        AssistantOperation::CreateThread => {
            threads::call(dispatcher, merged, request, ThreadOperation::Create).await
        }
        AssistantOperation::GetThread => {
            threads::call(dispatcher, merged, request, ThreadOperation::Get).await
        }
        AssistantOperation::CreateMessage => {
            messages::call(dispatcher, merged, MessageKind::Append).await
        }
        AssistantOperation::ListMessages => {
            messages::call(dispatcher, merged, MessageKind::List).await
        }
        AssistantOperation::RunThread => run::thread(dispatcher, merged, request).await,
    }
}
