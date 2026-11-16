use std::collections::BTreeMap;

use betterlite_models::decisions::openai::DecisionRequestBody;
use betterlite_openai::assistants::records::ListParams;
use betterlite_openai::realtime::url::QueryParams;
use betterlite_openai::thread_runs::records::RunRequest;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AssistantsOp {
    ListAssistants,
    CreateAssistant,
    DeleteAssistant,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThreadsOp {
    CreateThread,
    RetrieveThread,
    AddMessage,
    ListMessages,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunsOp {
    Buffered,
    Stream,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DecisionsOp {
    Request,
    Response,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RealtimeOp {
    Connect,
}

// Every operation name belongs to one family, so the name alone picks the family; the Python side routes on kind.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(untagged)]
pub(crate) enum Operation {
    Assistants(AssistantsOp),
    Threads(ThreadsOp),
    Runs(RunsOp),
    Decisions(DecisionsOp),
    Realtime(RealtimeOp),
}

// LiteLLM reads the verification from SSL_VERIFY at import, so the case sets both this field and the variable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SslVerification {
    #[default]
    Enabled,
    Disabled,
}

// One canned upstream answer: a JSON body, an SSE text body or an error status with either.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Reply {
    pub status: u16,
    pub body: Option<Value>,
    pub text: Option<String>,
    pub content_type: Option<String>,
}

// A case that lists fewer replies than the composition sends gets a server error for the rest.
impl Default for Reply {
    fn default() -> Self {
        Self {
            status: 500,
            body: None,
            text: None,
            content_type: None,
        }
    }
}

// A case names the fields its operation reads; the rest default, and neither side reads them.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Input {
    pub operation: Operation,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub params: ListParams,
    pub aget_assistants: Option<bool>,
    #[serde(default)]
    pub arun_thread: bool,
    #[serde(default)]
    pub data: Value,
    pub request: Option<RunRequest>,
    pub decision: Option<DecisionRequestBody>,
    #[serde(default)]
    pub payload: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub replies: Vec<Reply>,
    #[serde(default)]
    pub assistant_id: String,
    #[serde(default)]
    pub thread_id: String,
    pub query_params: Option<QueryParams>,
    #[serde(default)]
    pub ssl_verification: SslVerification,
    #[serde(default)]
    pub openai_beta: bool,
    pub close_error: Option<String>,
    pub handshake_status: Option<u16>,
    #[serde(default)]
    pub max_message_size_bytes: usize,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}
