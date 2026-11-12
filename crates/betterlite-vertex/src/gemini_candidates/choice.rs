use betterlite_openai::stream::chunk::Logprobs;
use serde::Serialize;

use crate::gemini_metadata::annotations::GroundingAnnotation;
use crate::gemini_response_parts::content::ThinkingBlock;
use crate::gemini_response_parts::media::{AudioPart, ImagePart};
use crate::gemini_response_parts::tools::{FunctionChunk, ServerToolInvocation, ToolCallChunk};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Assistant,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SpecificFields {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_signatures: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_side_tool_invocations: Option<Vec<ServerToolInvocation>>,
}

#[derive(Debug, Default, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub thinking_blocks: Option<Vec<ThinkingBlock>>,
    pub tool_calls: Option<Vec<ToolCallChunk>>,
    pub function_call: Option<FunctionChunk>,
    pub images: Option<Vec<ImagePart>>,
    pub audio: Option<AudioPart>,
    pub annotations: Option<Vec<GroundingAnnotation>>,
    pub provider_specific_fields: Option<SpecificFields>,
}

#[derive(Debug, Serialize)]
pub struct Choice {
    pub index: u32,
    pub finish_reason: &'static str,
    pub message: Message,
    pub logprobs: Option<Logprobs>,
    pub native_finish_reason: Option<String>,
}

// a streamed delta carries no thinking blocks or audio
#[derive(Debug, Default, Serialize)]
pub struct Delta {
    pub role: Option<Role>,
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<Vec<ToolCallChunk>>,
    pub function_call: Option<FunctionChunk>,
    pub images: Option<Vec<ImagePart>>,
    pub annotations: Option<Vec<GroundingAnnotation>>,
    pub provider_specific_fields: Option<SpecificFields>,
}

#[derive(Debug, Serialize)]
pub struct StreamChoice {
    pub index: u32,
    pub finish_reason: &'static str,
    pub delta: Delta,
    pub logprobs: Option<Logprobs>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Choices {
    Complete(Vec<Choice>),
    Streaming(Vec<StreamChoice>),
}
