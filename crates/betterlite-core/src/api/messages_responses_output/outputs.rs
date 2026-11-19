use betterlite_models::metadata::levels::ReasoningEffort;
use betterlite_prompts::arguments::json::Object;
use serde::Serialize;

use crate::api::messages_chat_output::output::MessageKind;
use crate::api::messages_responses_input::items::{InputItem, MessagePart};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ResponsesInputItem {
    Item(InputItem),
    Developer(DeveloperMessage),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeveloperMessage {
    pub r#type: MessageKind,
    pub role: DeveloperRole,
    pub content: Vec<MessagePart>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeveloperRole {
    Developer,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResponsesRequestRecord {
    pub model: String,
    pub input: Vec<ResponsesInputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ResponsesTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ResponsesToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ResponsesReasoning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<ResponsesText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_management: Option<Vec<ResponsesContextManagement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesTool {
    WebSearchPreview,
    Function {
        name: String,
        strict: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parameters: Option<Object>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ResponsesToolChoice {
    Mode(ToolChoiceMode),
    Function(ForcedFunction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoiceMode {
    Auto,
    Required,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ForcedFunction {
    pub r#type: ForcedFunctionKind,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForcedFunctionKind {
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponsesReasoning {
    pub effort: ReasoningEffort,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponsesText {
    pub format: ResponsesTextFormat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponsesTextFormat {
    pub r#type: StructuredOutputKind,
    pub name: &'static str,
    pub schema: Object,
    pub strict: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutputKind {
    JsonSchema,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponsesContextManagement {
    pub r#type: CompactionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_threshold: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionKind {
    Compaction,
}
