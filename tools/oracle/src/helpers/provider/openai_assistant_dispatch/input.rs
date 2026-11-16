use betterlite_models::decisions::openai::DecisionRequestBody;
use betterlite_openai::assistants::records::ListParams;
use betterlite_openai::realtime::url::QueryParams;
use betterlite_openai::thread_runs::records::RunRequest;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Assistants,
    Threads,
    Runs,
    Decisions,
    Realtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    AsyncList,
    Create,
    Delete,
    Retrieve,
    AddMessage,
    ListMessages,
    Buffered,
    Stream,
    Request,
    Response,
    Connect,
}

// LiteLLM reads the verification from SSL_VERIFY at import, so the case sets both this field and the variable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SslVerification {
    #[default]
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Params {
    pub order: Option<String>,
    pub limit: Option<i64>,
    pub before: Option<String>,
    pub after: Option<String>,
}

impl Params {
    pub(crate) fn list_params(&self) -> ListParams {
        ListParams {
            order: self.order.clone(),
            limit: self.limit,
            before: self.before.clone(),
            after: self.after.clone(),
        }
    }
}

// One canned upstream answer: a JSON body, an SSE text body or an error status with either.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Reply {
    pub status: u16,
    pub body: Option<Value>,
    pub text: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Input {
    pub kind: Kind,
    pub operation: Operation,
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    #[serde(default)]
    pub params: Params,
    pub aget_assistants: Option<bool>,
    #[serde(default)]
    pub arun_thread: bool,
    pub data: Option<Value>,
    pub request: Option<RunRequest>,
    pub decision: Option<DecisionRequestBody>,
    pub payload: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub replies: Vec<Reply>,
    pub assistant_id: Option<String>,
    pub thread_id: Option<String>,
    pub query_params: Option<QueryParams>,
    #[serde(default)]
    pub ssl_verification: SslVerification,
    #[serde(default)]
    pub openai_beta: bool,
    pub close_error: Option<String>,
    pub max_message_size_bytes: Option<usize>,
}
