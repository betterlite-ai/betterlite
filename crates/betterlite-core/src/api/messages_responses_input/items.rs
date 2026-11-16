use betterlite_prompts::reasoning_fields::replay_groups::ReasoningSummaryTextBlock;
use serde::Serialize;

use super::records::{MessageRole, PromptCacheBreakpoint};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputItem {
    Message {
        role: MessageRole,
        content: Vec<MessagePart>,
    },
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        call_id: String,
        output: ToolOutput,
    },
    Reasoning {
        summary: Vec<ReasoningSummaryTextBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        encrypted_content: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum MessagePart {
    #[serde(rename = "input_text")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
    },
    #[serde(rename = "input_image")]
    Image {
        image_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
    },
    #[serde(rename = "input_file")]
    FileData {
        filename: String,
        file_data: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
    },
    #[serde(rename = "input_file")]
    FileUrl {
        file_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
    },
    #[serde(rename = "output_text")]
    OutputText { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ToolOutput {
    Text(String),
    Parts(Vec<MessagePart>),
}
