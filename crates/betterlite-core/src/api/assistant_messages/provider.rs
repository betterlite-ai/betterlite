use std::sync::Arc;

use betterlite_azure::azure_assistant_definitions::operations::{
    AssistantsApi as AzureAssistantsApi, ClientRequest,
};
use betterlite_azure::azure_message_attachments::operations::{
    MessageCall, MessageReply, MessagesApi,
};
use betterlite_azure::credentials::source::CredentialSources;
use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings as AzureSettings;
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_openai::assistants::operations::AssistantsApi as OpenAiAssistantsApi;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::operations::{ClientSetup, ThreadCall, ThreadReply, ThreadsApi};
use betterlite_openai::threads::records::ThreadMessageData;
use betterlite_redact::secret::Redactor;

use crate::api::resource_sdk_settings::ResourceSdkSettings;

use super::connection::{AzureCredentials, Prepared, Route};
use super::error::MessageError;
use super::service::Reply;

pub struct MessageContext<'context, T> {
    pub settings: &'context ResourceSdkSettings,
    pub azure: &'context AzureSettings,
    pub environment: &'context (dyn EnvironmentLookup + Sync),
    pub sources: &'context (dyn CredentialSources + Sync),
    pub transport: Arc<T>,
    pub redactor: &'context Redactor,
}

pub enum ProviderCall<'call> {
    Append {
        thread_id: &'call str,
        data: &'call ThreadMessageData,
    },
    List(&'call str),
}

pub async fn send<T>(
    prepared: &Prepared,
    context: &MessageContext<'_, T>,
    call: ProviderCall<'_>,
) -> Result<Reply, MessageError>
where
    T: AssistantsTransport,
{
    match &prepared.route {
        // the openai facade: get_openai_client then beta.threads.messages.create or .list (openai.py#L2622-L2730)
        Route::OpenAi(credentials) => {
            let threads = ThreadsApi {
                assistants: OpenAiAssistantsApi {
                    transport: Arc::clone(&context.transport),
                },
            };
            let setup = ClientSetup {
                options: ClientOptions {
                    api_key: credentials.api_key.clone(),
                    api_base: Some(credentials.api_base.clone()),
                    timeout: Some(prepared.deadline),
                    max_retries: prepared.max_retries,
                    organization: credentials.organization.clone(),
                },
                supplied: None,
                async_requested: prepared.asynchronous,
            };
            match threads.call(setup, thread_call(call)).await? {
                ThreadReply::Message(message) => Ok(Reply::Message(message)),
                ThreadReply::Page(page) => Ok(Reply::Page(Box::new(page))),
                ThreadReply::Thread(_) => Err(MessageError::UnexpectedReply {
                    operation: "messages",
                }),
            }
        }
        Route::Azure(credentials) => azure_call(prepared, credentials, context, call).await,
    }
}

fn thread_call(call: ProviderCall<'_>) -> ThreadCall<'_> {
    match call {
        ProviderCall::Append { thread_id, data } => ThreadCall::AddMessage { thread_id, data },
        ProviderCall::List(thread_id) => ThreadCall::ListMessages(thread_id),
    }
}

// the azure facade: the same two SDK calls through initialize_azure_sdk_client (assistants.py#L190-L300)
async fn azure_call<T>(
    prepared: &Prepared,
    credentials: &AzureCredentials,
    context: &MessageContext<'_, T>,
    call: ProviderCall<'_>,
) -> Result<Reply, MessageError>
where
    T: AssistantsTransport,
{
    let messages = MessagesApi {
        assistants: AzureAssistantsApi {
            transport: Arc::clone(&context.transport),
        },
    };
    let request = ClientRequest {
        params: credentials.params.clone(),
        provider: None,
        api_key: credentials.api_key.clone(),
        api_base: credentials.api_base.clone(),
        api_version: credentials.api_version.clone(),
        azure_ad_token: credentials.facade_ad_token.clone(),
        timeout: prepared.deadline.as_secs_f64(),
        max_retries: None,
        refresh_enabled: false,
        session: SessionKind::Dedicated,
        settings: context.azure,
        environment: context.environment,
        sources: context.sources,
    };
    match messages
        .call(
            &request,
            None,
            Some(prepared.asynchronous),
            message_call(call),
        )
        .await?
        .reply
    {
        MessageReply::Message(message) => Ok(Reply::Message(message)),
        MessageReply::Page(page) => Ok(Reply::Page(page)),
    }
}

fn message_call(call: ProviderCall<'_>) -> MessageCall<'_> {
    match call {
        ProviderCall::Append { thread_id, data } => MessageCall::Append { thread_id, data },
        ProviderCall::List(thread_id) => MessageCall::List(thread_id),
    }
}
