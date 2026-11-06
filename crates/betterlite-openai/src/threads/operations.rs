use crate::assistants::client::{ClientKind, ClientOptions, SdkClient};
use crate::assistants::operations::AssistantsApi;
use crate::assistants::transport::AssistantsTransport;
use crate::threads::client;
use crate::threads::error::ThreadsError;
use crate::threads::records::{
    ThreadCreate, ThreadMessage, ThreadMessageData, ThreadMessagePage, ThreadRecord,
};

pub struct ClientSetup<T> {
    pub options: ClientOptions,
    pub supplied: Option<SdkClient<T>>,
    pub async_requested: bool,
}

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
    Message(Box<ThreadMessage>),
    Page(ThreadMessagePage),
}

pub struct ThreadsApi<T> {
    pub assistants: AssistantsApi<T>,
}

// LiteLLM selects the async branch only for the literal True flag, and every other value stays on the sync path.
const fn dispatch_kind(async_requested: bool) -> ClientKind {
    if async_requested {
        ClientKind::Async
    } else {
        ClientKind::Sync
    }
}

impl<T: AssistantsTransport> ThreadsApi<T> {
    #[must_use]
    pub fn client(&self, setup: ClientSetup<T>) -> SdkClient<T> {
        self.assistants.get_openai_client(
            dispatch_kind(setup.async_requested),
            setup.options,
            setup.supplied,
        )
    }

    pub async fn call(
        &self,
        setup: ClientSetup<T>,
        request: ThreadCall<'_>,
    ) -> Result<ThreadReply, ThreadsError> {
        let openai = self.client(setup);
        let reply = client::send(&openai, request);
        reply.await
    }
}
