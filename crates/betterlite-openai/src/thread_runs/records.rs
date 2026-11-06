use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::assistants::records::{AssistantTool, ResponseFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    InProgress,
    RequiresAction,
    Cancelling,
    Cancelled,
    Failed,
    Completed,
    Incomplete,
    Expired,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncompleteDetails {
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub arguments: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredActionToolCall {
    pub id: String,
    pub function: ToolCallFunction,
    pub r#type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitToolOutputs {
    pub tool_calls: Vec<RequiredActionToolCall>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredAction {
    pub submit_tool_outputs: SubmitToolOutputs,
    pub r#type: String,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruncationStrategy {
    pub r#type: String,
    pub last_messages: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunUsage {
    pub completion_tokens: i64,
    pub prompt_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoiceMode {
    None,
    Auto,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolChoiceFunction {
    pub name: String,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantToolChoice {
    pub r#type: String,
    pub function: Option<ToolChoiceFunction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoiceOption {
    Mode(ToolChoiceMode),
    Tool(AssistantToolChoice),
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReply {
    pub id: String,
    pub assistant_id: String,
    pub cancelled_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub failed_at: Option<i64>,
    pub incomplete_details: Option<IncompleteDetails>,
    pub instructions: String,
    pub last_error: Option<LastError>,
    pub max_completion_tokens: Option<i64>,
    pub max_prompt_tokens: Option<i64>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub model: String,
    pub object: String,
    pub parallel_tool_calls: bool,
    pub required_action: Option<RequiredAction>,
    pub response_format: Option<ResponseFormat>,
    pub started_at: Option<i64>,
    pub status: RunStatus,
    pub thread_id: String,
    pub tool_choice: Option<ToolChoiceOption>,
    pub tools: Vec<AssistantTool>,
    pub truncation_strategy: Option<TruncationStrategy>,
    pub usage: Option<RunUsage>,
    pub temperature: Option<Number>,
    pub top_p: Option<Number>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunErrorObject {
    pub code: Option<String>,
    pub message: String,
    pub param: Option<String>,
    pub r#type: String,
}

// The wire shape of one decoded run event: the SSE event name and its data object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum RunEvent {
    #[serde(rename = "thread.run.created")]
    Created(RunReply),
    #[serde(rename = "thread.run.queued")]
    Queued(RunReply),
    #[serde(rename = "thread.run.in_progress")]
    InProgress(RunReply),
    #[serde(rename = "thread.run.requires_action")]
    RequiresAction(RunReply),
    #[serde(rename = "thread.run.completed")]
    Completed(RunReply),
    #[serde(rename = "thread.run.incomplete")]
    Incomplete(RunReply),
    #[serde(rename = "thread.run.failed")]
    Failed(RunReply),
    #[serde(rename = "thread.run.cancelling")]
    Cancelling(RunReply),
    #[serde(rename = "thread.run.cancelled")]
    Cancelled(RunReply),
    #[serde(rename = "thread.run.expired")]
    Expired(RunReply),
    #[serde(rename = "error")]
    Error(RunErrorObject),
}

impl RunEvent {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Created(_) => "thread.run.created",
            Self::Queued(_) => "thread.run.queued",
            Self::InProgress(_) => "thread.run.in_progress",
            Self::RequiresAction(_) => "thread.run.requires_action",
            Self::Completed(_) => "thread.run.completed",
            Self::Incomplete(_) => "thread.run.incomplete",
            Self::Failed(_) => "thread.run.failed",
            Self::Cancelling(_) => "thread.run.cancelling",
            Self::Cancelled(_) => "thread.run.cancelled",
            Self::Expired(_) => "thread.run.expired",
            Self::Error(_) => "error",
        }
    }
}

// The keyword arguments LiteLLM's run wrappers receive; thread_id is the path and never the body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub thread_id: String,
    pub assistant_id: String,
    pub additional_instructions: Option<String>,
    pub instructions: Option<String>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub model: Option<String>,
    pub tools: Option<Vec<AssistantTool>>,
}

// A None argument reaches the provider as null, as it does through the SDK's transform.
#[derive(Debug, Serialize)]
pub(crate) struct RunCreate<'call> {
    pub(crate) assistant_id: &'call str,
    pub(crate) additional_instructions: Option<&'call str>,
    pub(crate) instructions: Option<&'call str>,
    pub(crate) metadata: Option<&'call BTreeMap<String, String>>,
    pub(crate) model: Option<&'call str>,
    pub(crate) stream: bool,
    pub(crate) tools: Option<&'call [AssistantTool]>,
}
