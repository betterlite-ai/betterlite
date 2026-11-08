use serde::{Deserialize, Serialize};

use crate::arguments::json::Json;
use crate::gemini_tool_invokes::types::GeminiTool;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LastToolCallMessage {
    // The assistant message declares tool_calls as an optional list; an absent or null list means none
    pub tool_calls: Option<Vec<GeminiTool>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GeminiInlineData {
    pub data: String,
    pub mime_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GeminiResponseMedia {
    pub inline_data: GeminiInlineData,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GeminiFunctionResponse {
    pub name: String,
    pub response: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<GeminiResponseMedia>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GeminiFunctionResponsePart {
    pub function_response: GeminiFunctionResponse,
}

// LiteLLM answers one part without media and a one-element list when media nests inside it
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum GeminiToolResult {
    Single(GeminiFunctionResponsePart),
    WithMedia(Vec<GeminiFunctionResponsePart>),
}
