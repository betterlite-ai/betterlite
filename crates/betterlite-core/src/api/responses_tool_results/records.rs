use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use super::error::ToolResultsError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageUrl {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ToolContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionChunk {
    pub name: String,
    pub arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolCallChunk {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionChunk,
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolMessage {
    pub role: &'static str,
    pub content: ToolContent,
    pub tool_call_id: String,
}

// LiteLLM leaves content out of the replayed call, unlike the one built from a function_call item
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CachedCallMessage {
    pub role: &'static str,
    pub tool_calls: Vec<ToolCallChunk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionCallMessage {
    pub role: &'static str,
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCallChunk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ResultMessage {
    CachedCall(CachedCallMessage),
    Tool(ToolMessage),
}

// an absent, null or empty value takes the default, as `value or default` did for a declared string
pub fn text_field<'fields>(
    fields: &'fields IndexMap<String, Json>,
    field: &'static str,
    default: &'fields str,
) -> Result<&'fields str, ToolResultsError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(default),
        Some(Json::Text(text)) if text.is_empty() => Ok(default),
        Some(Json::Text(text)) => Ok(text),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ToolResultsError::NotText { field })
        }
    }
}
