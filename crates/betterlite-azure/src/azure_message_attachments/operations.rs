use betterlite_http::client_kind::ClientKind;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::records::{ThreadMessage, ThreadMessageData, ThreadMessagePage};
use http::Method;

use crate::azure_assistant_definitions::client::AzureAssistantsClient;
use crate::azure_assistant_definitions::operations::{AssistantsApi, ClientRequest};
use crate::azure_message_attachments::client::{append_message, send};
use crate::azure_message_attachments::error::MessagesError;

pub struct MessagesApi<T> {
    pub assistants: AssistantsApi<T>,
}

// What one facade call asks the SDK client to do.
pub enum MessageCall<'call> {
    Append {
        thread_id: &'call str,
        data: &'call ThreadMessageData,
    },
    List(&'call str),
}

// The SDK object a facade call returns, each one exactly as the SDK built it.
#[derive(Debug)]
pub enum MessageReply {
    Message(Box<ThreadMessage>),
    Page(Box<ThreadMessagePage>),
}

// The SDK's reply stays untouched; the kind names which SDK client the call chose.
#[derive(Debug)]
pub struct MessagesReply {
    pub reply: MessageReply,
    pub kind: ClientKind,
}

impl<T: AssistantsTransport> MessagesApi<T> {
    pub async fn call<'data>(
        &self,
        request: &ClientRequest<'data>,
        supplied: Option<AzureAssistantsClient<'data, T>>,
        flag: Option<bool>,
        message: MessageCall<'_>,
    ) -> Result<MessagesReply, MessagesError> {
        let client = self
            .assistants
            .azure_client(request, supplied, requested_kind(flag))?;
        let kind = client.kind;
        let reply = match message {
            MessageCall::Append { thread_id, data } => {
                MessageReply::Message(Box::new(append_message(&client, thread_id, data).await?))
            }
            MessageCall::List(thread_id) => MessageReply::Page(Box::new(
                send(&client, Method::GET, thread_id, RequestBody::Empty).await?,
            )),
        };
        Ok(MessagesReply { reply, kind })
    }
}

// Only the literal true flag selects the async client; a missing, false or other facade value stays sync (assistants.py#L283-L284).
const fn requested_kind(flag: Option<bool>) -> ClientKind {
    match flag {
        Some(true) => ClientKind::Asynchronous,
        Some(false) | None => ClientKind::Sync,
    }
}
