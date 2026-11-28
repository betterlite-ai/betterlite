use std::sync::Arc;

use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::records::{ThreadCreate, ThreadRecord};

use crate::azure_assistant_definitions::client::AzureAssistantsClient;
use crate::azure_assistant_definitions::operations::{AssistantsApi, ClientRequest};
use crate::azure_thread_messages::error::ThreadMessagesError;
use crate::azure_thread_messages::request::send;

pub struct ThreadMessagesApi<T> {
    pub transport: Arc<T>,
}

// What one facade call asks the SDK's beta.threads namespace to do.
pub enum ThreadOperation<'call> {
    Create(&'call ThreadCreate),
    Retrieve(&'call str),
}

// The SDK's thread stays untouched; kind names which client the call chose, as AssistantsReply does.
#[derive(Debug)]
pub struct ThreadReply {
    pub thread: ThreadRecord,
    pub kind: ClientKind,
}

impl<T> ThreadMessagesApi<T>
where
    T: AssistantsTransport,
{
    // Serves both thread facades: the flag picks the client kind and the operation picks the SDK method.
    pub async fn call<'data>(
        &self,
        request: &ClientRequest<'data>,
        supplied: Option<AzureAssistantsClient<'data, T>>,
        flag: Option<bool>,
        operation: ThreadOperation<'_>,
    ) -> Result<ThreadReply, ThreadMessagesError> {
        let assistants = AssistantsApi {
            transport: Arc::clone(&self.transport),
        };
        let client = assistants.azure_client(request, supplied, requested_kind(flag))?;
        let kind = client.kind;
        let thread = send(&client, operation).await?;
        Ok(ThreadReply { thread, kind })
    }
}

// Only the literal true flag selects the async client; a missing, false or other facade value stays sync.
const fn requested_kind(flag: Option<bool>) -> ClientKind {
    match flag {
        Some(true) => ClientKind::Asynchronous,
        Some(false) | None => ClientKind::Sync,
    }
}
