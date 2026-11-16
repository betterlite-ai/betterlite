use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use serde_json::Number;

use crate::databricks_chat::content::{ThinkingBlock, extract_content_str, extract_reasoning};
use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::response::ReplyFunction;
use crate::databricks_chat::slot::Slot;
use crate::databricks_chat::stream::wire::{
    CustomPayload, DatabricksDelta, DeltaFunctionCall, DeltaToolCall,
};

// LiteLLM's Delta: the five OpenAI keys always dump, the rest only when they carry something
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    #[serde(skip_serializing_if = "Slot::is_absent")]
    pub provider_specific_fields: Slot<Object>,
}

// chunk_parser rewrites content, reasoning_content and thinking_blocks, then Delta(**delta) reads it
pub(super) fn build_delta(delta: DatabricksDelta) -> Result<Delta, DatabricksChatError> {
    let content = extract_content_str(delta.content.as_ref());
    let (block_reasoning, thinking_blocks) = extract_reasoning(delta.content.as_ref());
    let mut extra = delta.extra;
    extra.shift_remove("thinking_blocks");
    // Delta maps reasoning onto reasoning_content only when chunk_parser left it unset
    let mut reasoning_content = block_reasoning.or(delta.reasoning_content);
    match (&delta.reasoning, reasoning_content.is_some()) {
        (Slot::Absent, _) | (Slot::Null, false) => {}
        (Slot::Value(text), true) => {
            extra.insert(String::from("reasoning"), Json::Text(text.clone()));
        }
        (Slot::Null, true) => {
            extra.insert(String::from("reasoning"), Json::Null);
        }
        (Slot::Value(text), false) => reasoning_content = Some(text.clone()),
    }
    Ok(Delta {
        extra,
        content,
        role: delta.role,
        function_call: delta.function_call,
        tool_calls: delta.tool_calls.map(normalize).transpose()?,
        audio: delta.audio,
        images: delta.images.filter(|images| !images.is_empty()),
        annotations: delta.annotations,
        reasoning_items: delta.reasoning_items,
        reasoning_content,
        thinking_blocks,
        provider_specific_fields: delta.provider_specific_fields,
    })
}

// Delta coerces each tool call into a function call or, for a custom one, a custom call
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum DeltaCall {
    Function(FunctionDeltaCall),
    Custom(CustomDeltaCall),
}

// ChatCompletionDeltaToolCall keeps the call keys it does not name, and an absent id stays absent
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionDeltaCall {
    #[serde(flatten)]
    pub extra: Object,
    pub function: ReplyFunction,
    #[serde(skip_serializing_if = "Slot::is_absent")]
    pub id: Slot<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub index: Number,
}

// ChatCompletionDeltaCustomToolCall: an unset id or type stays absent, a null one dumps as null
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CustomDeltaCall {
    #[serde(flatten)]
    pub extra: Object,
    #[serde(skip_serializing_if = "Slot::is_absent")]
    pub id: Slot<String>,
    #[serde(rename = "type", skip_serializing_if = "Slot::is_absent")]
    pub kind: Slot<String>,
    pub custom: CustomPayload,
    pub index: Number,
}

// A call without an index gets the next one, counting from zero over the calls that had none
fn normalize(calls: Vec<DeltaToolCall>) -> Result<Vec<DeltaCall>, DatabricksChatError> {
    let mut next_index = 0_i64;
    let mut normalized = Vec::with_capacity(calls.len());
    for call in calls {
        let index = call.index.clone().unwrap_or_else(|| {
            let assigned = Number::from(next_index);
            next_index = next_index.saturating_add(1);
            assigned
        });
        normalized.push(coerced(call, index)?);
    }
    Ok(normalized)
}

// is_custom_tool_call_dict picks the custom shape; every other call needs its function
fn coerced(call: DeltaToolCall, index: Number) -> Result<DeltaCall, DatabricksChatError> {
    let named_kind = match &call.kind {
        Slot::Value(kind) => Some(kind.clone()),
        Slot::Null | Slot::Absent => None,
    };
    if named_kind.as_deref() == Some("custom") || call.custom.is_some() {
        let Some(custom) = call.custom else {
            return Err(DatabricksChatError::Shape {
                message: String::from("a custom delta tool call is missing its custom payload"),
            });
        };
        return Ok(DeltaCall::Custom(CustomDeltaCall {
            extra: call.extra,
            id: call.id,
            kind: call.kind,
            custom,
            index,
        }));
    }
    let Slot::Value(function) = call.function else {
        return Err(DatabricksChatError::Shape {
            message: String::from("a delta tool call is missing its function"),
        });
    };
    Ok(DeltaCall::Function(FunctionDeltaCall {
        extra: call.extra,
        function: ReplyFunction {
            arguments: function.arguments_text(),
            name: function.name,
        },
        id: call.id,
        kind: named_kind.unwrap_or_else(|| String::from("function")),
        index,
    }))
}
