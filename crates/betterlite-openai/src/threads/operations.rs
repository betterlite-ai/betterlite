use crate::assistants::client::{ClientKind, ClientOptions, SdkClient};
use crate::assistants::operations::AssistantsApi;
use crate::assistants::transport::AssistantsTransport;
use crate::threads::client;
use crate::threads::error::ThreadsError;
use crate::threads::records::{
    ThreadCreate, ThreadMessage, ThreadMessageData, ThreadMessagePage, ThreadRecord,
};

#[derive(Debug)]
pub enum ThreadCall<'request> {
    Create(&'request ThreadCreate),
    Retrieve(&'request str),
    AddMessage {
        thread_id: &'request str,
        data: &'request ThreadMessageData,
    },
    ListMessages(&'request str),
}

#[derive(Debug)]
pub enum ThreadReply {
    Thread(ThreadRecord),
    Message(ThreadMessage),
    Page(ThreadMessagePage),
}

pub struct ThreadsApi<T> {
    pub assistants: AssistantsApi<T>,
}

// LiteLLM selects the async branch only for the literal True flag, and every other value stays on the sync path.
#[must_use]
pub const fn dispatch_kind(async_requested: bool) -> ClientKind {
    if async_requested {
        ClientKind::Async
    } else {
        ClientKind::Sync
    }
}

impl<T: AssistantsTransport> ThreadsApi<T> {
    pub async fn call(
        &self,
        options: ClientOptions,
        client: Option<SdkClient<T>>,
        async_requested: bool,
        request: ThreadCall<'_>,
    ) -> Result<ThreadReply, ThreadsError> {
        let kind = dispatch_kind(async_requested);
        let openai = self.assistants.get_openai_client(kind, options, client);
        client::send(&openai, request).await
    }
}
