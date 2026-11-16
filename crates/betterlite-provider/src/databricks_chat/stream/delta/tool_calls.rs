use betterlite_prompts::arguments::json::Object;
use serde::Serialize;
use serde_json::Number;

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::response::reply::ReplyFunction;
use crate::databricks_chat::stream::wire::{CustomPayload, DeltaToolCall};

// Delta coerces each tool call into a function call or, for a custom one, a custom call
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum DeltaCall {
    Function(FunctionDeltaCall),
    Custom(CustomDeltaCall),
}

// ChatCompletionDeltaToolCall keeps the call keys it does not name
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FunctionDeltaCall {
    pub id: Option<String>,
    pub function: ReplyFunction,
    #[serde(rename = "type")]
    pub kind: String,
    pub index: Number,
    #[serde(flatten)]
    pub extra: Object,
}

// ChatCompletionDeltaCustomToolCall: the type stays null when the call names none
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CustomDeltaCall {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub custom: CustomPayload,
    pub index: Number,
    #[serde(flatten)]
    pub extra: Object,
}

// A call without an index gets the next one, counting from zero over the calls that had none
pub(super) fn normalize(calls: Vec<DeltaToolCall>) -> Result<Vec<DeltaCall>, DatabricksChatError> {
    let mut next_index = 0_i64;
    let mut normalized = Vec::with_capacity(calls.len());
    for call in calls {
        let assigned = call.index.is_none();
        let index = call
            .index
            .clone()
            .unwrap_or_else(|| Number::from(next_index));
        if assigned {
            next_index = next_index.saturating_add(1);
        }
        normalized.push(coerced(call, index)?);
    }
    Ok(normalized)
}

// is_custom_tool_call_dict picks the custom shape; every other call needs its function
fn coerced(call: DeltaToolCall, index: Number) -> Result<DeltaCall, DatabricksChatError> {
    if call.kind.as_deref() == Some("custom") || call.custom.is_some() {
        let Some(custom) = call.custom else {
            return Err(DatabricksChatError::Shape {
                message: String::from("a custom delta tool call is missing its custom payload"),
            });
        };
        return Ok(DeltaCall::Custom(CustomDeltaCall {
            id: call.id,
            kind: call.kind,
            custom,
            index,
            extra: call.extra,
        }));
    }
    let Some(function) = call.function else {
        return Err(DatabricksChatError::Shape {
            message: String::from("a delta tool call is missing its function"),
        });
    };
    Ok(DeltaCall::Function(FunctionDeltaCall {
        id: call.id,
        function: ReplyFunction {
            arguments: function.arguments_text()?,
            name: function.name,
        },
        kind: call.kind.unwrap_or_else(|| String::from("function")),
        index,
        extra: call.extra,
    }))
}
