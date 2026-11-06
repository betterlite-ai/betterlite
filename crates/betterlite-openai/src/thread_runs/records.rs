use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::assistants::records::AssistantTool;
use crate::thread_runs::events::{RunMessage, RunMessageDeltaEvent, RunStep, RunStepDeltaEvent};

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
// A declared field the provider left out stays null in the reply, as the SDK model keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncompleteDetails {
    pub reason: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastError {
    pub code: String,
    pub message: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub arguments: String,
    pub name: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredActionToolCall {
    pub id: String,
    pub function: ToolCallFunction,
    pub r#type: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitToolOutputs {
    pub tool_calls: Vec<RequiredActionToolCall>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredAction {
    pub submit_tool_outputs: SubmitToolOutputs,
    pub r#type: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruncationStrategy {
    pub r#type: String,
    pub last_messages: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunUsage {
    pub completion_tokens: i64,
    pub prompt_tokens: i64,
    pub total_tokens: i64,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
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
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantToolChoice {
    pub r#type: String,
    pub function: Option<ToolChoiceFunction>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoiceOption {
    Mode(ToolChoiceMode),
    Tool(AssistantToolChoice),
}
// The reply keeps the SDK's tool dump: a declared key the provider left out stays null and extras stay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Value")]
pub struct RunTool(Value);
impl From<Value> for RunTool {
    fn from(mut value: Value) -> Self {
        normalise_tool(&mut value);
        Self(value)
    }
}
fn normalise_tool(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    match object.get("type").and_then(Value::as_str) {
        Some("file_search") => {
            object.entry("file_search").or_insert(Value::Null);
        }
        Some("function") => {
            if let Some(Value::Object(function)) = object.get_mut("function") {
                for key in ["description", "parameters", "strict"] {
                    function.entry(key).or_insert(Value::Null);
                }
            }
        }
        _ => {}
    }
}
// The SDK dumps a JSON schema format under its field name, so the reply renames schema to schema_.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Value")]
pub struct RunResponseFormat(Value);
impl From<Value> for RunResponseFormat {
    fn from(mut value: Value) -> Self {
        normalise_format(&mut value);
        Self(value)
    }
}
fn normalise_format(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if object.get("type").and_then(Value::as_str) != Some("json_schema") {
        return;
    }
    let Some(Value::Object(schema)) = object.get_mut("json_schema") else {
        return;
    };
    for key in ["description", "strict"] {
        schema.entry(key).or_insert(Value::Null);
    }
    if let Some(item) = schema.remove("schema") {
        schema.insert(String::from("schema_"), item);
    }
    schema.entry("schema_").or_insert(Value::Null);
}
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
    pub response_format: Option<RunResponseFormat>,
    pub started_at: Option<i64>,
    pub status: RunStatus,
    pub thread_id: String,
    pub tool_choice: Option<ToolChoiceOption>,
    pub tools: Vec<RunTool>,
    pub truncation_strategy: Option<TruncationStrategy>,
    pub usage: Option<RunUsage>,
    pub temperature: Option<Number>,
    pub top_p: Option<Number>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
// Run, step and message events carry a typed record; other thread.* events stay raw or bare.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum RunEvent {
    Typed {
        event: String,
        data: Box<RunReply>,
    },
    Message {
        event: String,
        data: Box<RunMessage>,
    },
    MessageDelta {
        event: String,
        data: Box<RunMessageDeltaEvent>,
    },
    Step {
        event: String,
        data: Box<RunStep>,
    },
    StepDelta {
        event: String,
        data: Box<RunStepDeltaEvent>,
    },
    Named {
        event: String,
        data: Value,
    },
    Bare(Value),
}
impl RunEvent {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Typed { event, .. }
            | Self::Message { event, .. }
            | Self::MessageDelta { event, .. }
            | Self::Step { event, .. }
            | Self::StepDelta { event, .. }
            | Self::Named { event, .. } => event,
            Self::Bare(_) => "payload",
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
