use std::collections::BTreeMap;

use betterlite_config::tls::Verification;
use betterlite_models::decisions::openai::DecisionRequestBody;
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_openai::assistants::records::{AssistantCreate, ListParams};
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::realtime::failure::RealtimeFailure;
use betterlite_openai::realtime::url::QueryParams;
use betterlite_openai::thread_runs::operations::RunDispatch;
use betterlite_openai::thread_runs::records::RunRequest;
use betterlite_openai::threads::operations::ThreadCall;
use betterlite_openai::threads::records::{ThreadCreate, ThreadMessageData};
use secrecy::SecretString;

// The typed call the composition executes; operation.rs carries the unit marker above it and the caller keeps the call beside it.
#[derive(Debug)]
pub enum OpenAiAssistantOperation {
    Assistants(AssistantsCall),
    Threads(ThreadsCall),
    Runs(RunsCall),
    Decisions(DecisionsCall),
    Realtime(RealtimeCall),
}

// The assistant listing reaches the facade method or async_get_assistants directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantsFacade {
    AsyncHelper,
    Facade { aget_assistants: bool },
}

#[derive(Debug, Clone)]
pub enum AssistantsOperation {
    List {
        facade: AssistantsFacade,
        params: ListParams,
    },
    // the create record dwarfs the listing parameters, so it is boxed to stay within 3x, the limit variant_size_differences enforces
    Create {
        data: Box<AssistantCreate>,
        async_requested: bool,
    },
    Delete {
        assistant_id: String,
        async_requested: bool,
    },
}

#[derive(Debug, Clone)]
pub struct AssistantsCall {
    pub options: ClientOptions,
    pub operation: AssistantsOperation,
}

// the message record dwarfs the thread calls, so it is boxed to stay within 3x, the limit variant_size_differences enforces
#[derive(Debug, Clone)]
pub enum ThreadsOperation {
    Create(ThreadCreate),
    Retrieve(String),
    AddMessage(String, Box<ThreadMessageData>),
    ListMessages(String),
}

impl ThreadsOperation {
    // The leaf reads a borrowed view of the operation, so the owned call lends it out here.
    #[must_use]
    pub fn leaf_call(&self) -> ThreadCall<'_> {
        match self {
            Self::Create(data) => ThreadCall::Create(data),
            Self::Retrieve(thread_id) => ThreadCall::Retrieve(thread_id),
            Self::AddMessage(thread_id, data) => ThreadCall::AddMessage { thread_id, data },
            Self::ListMessages(thread_id) => ThreadCall::ListMessages(thread_id),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ThreadsCall {
    pub options: ClientOptions,
    pub async_requested: bool,
    pub operation: ThreadsOperation,
}

#[derive(Debug, Clone)]
pub struct RunsCall {
    pub options: ClientOptions,
    pub dispatch: RunDispatch,
    pub request: RunRequest,
}

#[derive(Debug, Clone)]
pub enum DecisionsOperation {
    Request {
        model: String,
        request: DecisionRequestBody,
    },
    Response {
        model: String,
        request: DecisionRequestBody,
        payload: Vec<u8>,
    },
}

// An explicit api_base or api_key wins, then the caller's LiteLLM globals, then the caller's environment map; the leaf config answers the default.
#[derive(Debug)]
pub struct DecisionsCall {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub globals: LiteLlmGlobals,
    pub environment: BTreeMap<String, String>,
    pub operation: DecisionsOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealtimeFailureOwned {
    Handshake {
        status_code: u16,
    },
    Other {
        message: String,
    },
    CloseFailed {
        message: String,
        close_error: String,
    },
}

impl RealtimeFailureOwned {
    #[must_use]
    pub fn borrowed(&self) -> RealtimeFailure<'_> {
        match self {
            Self::Handshake { status_code } => RealtimeFailure::Handshake {
                status_code: *status_code,
            },
            Self::Other { message } => RealtimeFailure::Other { message },
            Self::CloseFailed {
                message,
                close_error,
            } => RealtimeFailure::CloseFailed {
                message,
                close_error,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum RealtimeCall {
    Connect {
        api_base: Option<String>,
        api_key: Option<SecretString>,
        model: String,
        query_params: Option<QueryParams>,
        beta_realtime: bool,
        ssl_verification: Verification,
        max_message_size_bytes: usize,
    },
    Failure(RealtimeFailureOwned),
}
