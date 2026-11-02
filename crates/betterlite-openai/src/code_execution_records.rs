pub mod chat_dialect;
pub mod code_parsing;
pub mod container_extraction;
pub mod followup_filtering;
pub mod helpers;
pub mod metadata_filtering;
pub mod responses_dialect;
pub mod tool_calls;
pub mod tool_shapes;

pub const CODE_EXECUTION_TOOL_NAME: &str = "litellm_code_execution";

pub const INTERCEPTION_ACTIVE_KEY: &str = "_code_interpreter_interception_active";
pub const SANDBOX_KEY: &str = "_code_interpreter_interception_sandbox_key";
pub const SESSION_SCOPED_KEY: &str = "_code_interpreter_interception_session_scoped";
pub const CONVERTED_STREAM_KEY: &str = "_code_interpreter_interception_converted_stream";

// LiteLLM strips every interceptor's own markers, not only this one's, when a fresh request arrives
pub const INTERCEPTION_INTERNAL_PREFIXES: [&str; 4] = [
    "_code_interpreter_interception",
    "_websearch_interception",
    "_compression_interception",
    "_headroom_interception",
];

// The agentic loop keeps this interceptor's own markers across a follow-up call and drops the others
pub const OTHER_INTERCEPTION_PREFIXES: [&str; 3] = [
    "_websearch_interception",
    "_compression_interception",
    "_headroom_interception",
];

// LiteLLM picks the Responses or Chat Completions tool and tool_choice shape by call type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDialect {
    Responses,
    ChatCompletion,
}

// Mirrors LiteLLM's CodeExecutionToolCall TypedDict(total=False): a field absent there is None here
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeExecutionToolCall {
    pub id: Option<String>,
    pub call_id: Option<String>,
    pub kind: Option<String>,
    pub name: String,
    pub arguments: String,
}

include!("code_execution_records/tests/support.rs");
