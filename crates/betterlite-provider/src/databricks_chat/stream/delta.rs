use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use crate::databricks_chat::content::{ThinkingBlock, extract_content_str, extract_reasoning};
use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::keyed::Keyed;
use crate::databricks_chat::stream::delta::tool_calls::DeltaCall;
use crate::databricks_chat::stream::wire::{DatabricksDelta, DeltaFunctionCall};

pub mod tool_calls;

// LiteLLM's Delta: the five OpenAI keys always dump, the rest only when they carry something
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Delta {
    #[serde(flatten)]
    pub extra: Object,
    pub content: Option<String>,
    pub role: Option<String>,
    pub function_call: Option<DeltaFunctionCall>,
    pub tool_calls: Option<Vec<DeltaCall>>,
    pub audio: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_items: Option<Vec<Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_blocks: Option<Vec<ThinkingBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<Object>,
}

// chunk_parser rewrites content, reasoning_content and thinking_blocks, then Delta(**delta) reads it
pub(super) fn build_delta(delta: DatabricksDelta) -> Result<Delta, DatabricksChatError> {
    let content = extract_content_str(delta.content.as_ref());
    let (block_reasoning, thinking_blocks) = extract_reasoning(delta.content.as_ref());
    let mut extra = delta.extra;
    extra.shift_remove("thinking_blocks");
    // Delta maps reasoning onto reasoning_content only when chunk_parser left that unset
    let reasoning_content = match (block_reasoning.or(delta.reasoning_content), delta.reasoning) {
        (Some(resolved), Keyed::Given(reasoning)) => {
            extra.insert(String::from("reasoning"), Json::Text(reasoning));
            Some(resolved)
        }
        (Some(resolved), Keyed::Null) => {
            extra.insert(String::from("reasoning"), Json::Null);
            Some(resolved)
        }
        (None, Keyed::Given(reasoning)) => Some(reasoning),
        (resolved, Keyed::Missing) | (resolved @ None, Keyed::Null) => resolved,
    };
    Ok(Delta {
        extra,
        content,
        role: delta.role,
        function_call: delta.function_call,
        tool_calls: delta.tool_calls.map(tool_calls::normalize).transpose()?,
        audio: delta.audio,
        images: delta.images.filter(|images| !images.is_empty()),
        annotations: delta.annotations,
        reasoning_items: delta.reasoning_items,
        reasoning_content,
        thinking_blocks,
        provider_specific_fields: delta.provider_specific_fields,
    })
}
