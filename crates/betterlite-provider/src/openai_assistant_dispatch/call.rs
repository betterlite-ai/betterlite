use betterlite_config::tls::Verification;
use betterlite_models::decisions::openai::DecisionRequestBody;
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_openai::assistants::records::{AssistantCreate, ListParams};
use betterlite_openai::realtime::failure::RealtimeFailure;
use betterlite_openai::realtime::url::QueryParams;
use betterlite_openai::thread_runs::operations::RunDispatch;
use betterlite_openai::thread_runs::records::RunRequest;
use betterlite_openai::threads::records::{ThreadCreate, ThreadMessageData};
use secrecy::SecretString;

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
    Create {
        data: AssistantCreate,
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

#[derive(Debug, Clone)]
pub enum ThreadsOperation {
    Create {
        data: ThreadCreate,
        async_requested: bool,
    },
    Retrieve {
        thread_id: String,
        async_requested: bool,
    },
    AddMessage {
        thread_id: String,
        data: ThreadMessageData,
        async_requested: bool,
    },
    ListMessages {
        thread_id: String,
        async_requested: bool,
    },
}

#[derive(Debug, Clone)]
pub struct ThreadsCall {
    pub options: ClientOptions,
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

// An already resolved api_base or api_key wins; otherwise the leaf config resolves the default or the key.
#[derive(Debug, Clone)]
pub struct DecisionsCall {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub operation: DecisionsOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealtimeFailureOwned {
    Handshake { status_code: u16 },
    Other { message: String },
    CloseFailed { message: String, close_error: String },
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
