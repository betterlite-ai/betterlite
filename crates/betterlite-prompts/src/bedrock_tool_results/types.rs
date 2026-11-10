use serde::Serialize;

use crate::arguments::json::Object;
use crate::bedrock_images::types::{DocumentBlock, ImageBlock};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolUseBlock {
    pub input: Object,
    pub name: String,
    #[serde(rename = "toolUseId")]
    pub tool_use_id: String,
}

// LiteLLM's ContentBlock is a TypedDict with optional members; one tool call fills either member
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ToolInvokeBlock {
    ToolUse {
        #[serde(rename = "toolUse")]
        tool_use: ToolUseBlock,
    },
    CachePoint {
        #[serde(rename = "cachePoint")]
        cache_point: Object,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolResultStatus {
    Success,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ToolResultPart {
    Text {
        text: String,
    },
    Image {
        image: ImageBlock,
    },
    Document {
        document: DocumentBlock,
    },
    SearchResult {
        #[serde(rename = "searchResult")]
        search_result: Object,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolResultBlock {
    pub content: Vec<ToolResultPart>,
    #[serde(rename = "toolUseId")]
    pub tool_use_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ToolResultStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolResultContentBlock {
    #[serde(rename = "toolResult")]
    pub tool_result: ToolResultBlock,
}
