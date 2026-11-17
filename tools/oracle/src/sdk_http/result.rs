use betterlite_core::api::messages_sse_events::stream::MessagesStreamHiddenParams;
use betterlite_openai::embedding::EmbeddingResponse;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use serde_with::rust::double_option;

use super::messages_stream_logging::outcome::{Flag, StreamLoggingRun};
use super::messages_stream_logging::relay::call::StepOutcome;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SdkResult {
    Success { value: ChatCompletion },
    Embedding { value: EmbeddingOutcomes },
    Vectors { value: QueryVectors },
    BetaConfig { value: BetaConfigRun },
    MessagesHeaderStream { value: HeaderStreamRun },
    MessagesStreamLogging { value: StreamLoggingRun },
    MessagesNativeStream { value: Vec<StepOutcome> },
    MessagesSyncStream { value: SyncRun },
    MessagesChatStream { value: ChatStreamRun },
    ChatResponseStream { value: ChatResponseStreamRuns },
    ResponsesAgenticContinuation { value: AgenticContinuationRun },
    FileAzureStorage { value: FileStorageRun },
    Failure(Failure),
}

// the mock origin an upload would echo is stripped, so both sides report the same blob-origin path
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileStorageRun {
    pub url: Option<String>,
    pub downloaded: Option<Vec<u8>>,
    pub error: Option<FileStorageFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileStorageFailure {
    pub message: String,
    pub status: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgenticContinuationRun {
    pub returned: Option<AgenticIdentity>,
    pub error: Option<AgenticFailure>,
}

// the provider answer carries the whole response object, so only these two fields are read
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgenticIdentity {
    pub created_at: i64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgenticFailure {
    pub class: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncRun {
    pub runs: Vec<SyncRunOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncRunOutcome {
    pub steps: Vec<SyncStepOutcome>,
}

// One driver call: what it handed back, whether the iteration ended, and the state it left
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncStepOutcome {
    pub events: Vec<Value>,
    pub end: bool,
    pub state: SyncState,
    pub diagnostics: Vec<SyncDiagnostic>,
}

// LiteLLM's public stream attributes, so a seeded branch is observed the same way on both sides
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncState {
    pub pulls: usize,
    pub index: i64,
    pub block_type: String,
    pub sent_first_chunk: Flag,
    pub sent_content_block_start: Flag,
    pub sent_content_block_finish: Flag,
    pub sent_last_message: Flag,
    pub queued_usage_chunk: Flag,
    pub holding_chunk: Flag,
    pub holding_stop_reason: Flag,
    pub sent_compaction_block: Flag,
    pub sent_compaction_block_start: Flag,
    pub sent_compaction_block_delta: Flag,
}

// The swallowed failure's level and message prefix, since the exception text differs by runtime
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncDiagnostic {
    pub level: String,
    pub prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatStreamRun {
    pub runs: Vec<ChatStreamRunOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatStreamRunOutcome {
    pub steps: Vec<ChatStreamStepOutcome>,
}

// One __anext__ of the mode under test: its events or frames, the failure it raised, and what it left behind
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatStreamStepOutcome {
    pub events: Vec<Value>,
    pub frames: Vec<String>,
    pub end: bool,
    pub failure: Option<ChatStreamFailure>,
    pub pulls: usize,
    pub diagnostics: Vec<SyncDiagnostic>,
    pub view: Option<ChatStreamView>,
}

// What a caller of __anext__ sees: a reported status and message, or the bare exception's text
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatStreamFailure {
    pub kind: String,
    pub status_code: Option<u16>,
    pub message: String,
}

// The wrapper metadata a byte view forwards, with the chunk ids so the sequence itself is compared
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatStreamView {
    pub model: String,
    pub chunks: Option<Vec<String>>,
}

type ChatResponseStreamRuns = Vec<ChatResponseStreamRunOutcome>;

// One run of the Responses iterator: its steps, or the provider failure that ended it before any event
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatResponseStreamRunOutcome {
    pub steps: Vec<ChatResponseStreamStepOutcome>,
    pub failure: Option<Failure>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatResponseStreamStepOutcome {
    pub events: Vec<Value>,
    pub end: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderStreamRun {
    pub hidden_params: MessagesStreamHiddenParams,
    pub provider_bytes: Vec<u8>,
    pub has_buffered_provider_output: bool,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingOutcomes {
    pub sync: EmbeddingResponse,
    #[serde(rename = "async")]
    pub asynchronous: EmbeddingResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryVectors {
    pub sync: Vec<Number>,
    #[serde(rename = "async")]
    pub asynchronous: Vec<Number>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BetaConfigRun {
    pub steps: Vec<BetaStepOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BetaStepOutcome {
    pub matrix: Value,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub level: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatCompletion {
    pub id: String,
    pub created: u64,
    pub object: String,
    pub model: Option<String>,
    pub choices: Vec<Choice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub content: Option<String>,
    pub finish_reason: Option<String>,
}

// None: key absent, Some(None): JSON null, Some(Some(text)): text
pub type Nullable = Option<Option<String>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub class: String,
    pub status: u16,
    #[serde(
        default,
        rename = "type",
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub kind: Nullable,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub param: Nullable,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub code: Nullable,
    pub message: String,
}
