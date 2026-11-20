#![expect(
    clippy::disallowed_types,
    reason = "Anthropic request bodies, scan targets and guardrail-returned tool calls are provider JSON that betterlite does not model"
)]

pub mod error;
mod tests;

use serde_json::{Map, Value};

use betterlite_guardrails::guardrail_message_scope::unappliable_rewrite::UnappliableRequestRewrite;

use crate::guardrails::anthropic_translation::scan_targets::{
    InputWriteBackTarget, MessageTextTarget, ScannedText, ScannedToolCall, ToolUseInputTarget,
};
use crate::guardrails::tool_call_identity::{ToolCallShape, tool_call_shapes};

use error::{ApplyError, WriteBackError};

const CONTENT: &str = "content";
const INPUT: &str = "input";
const MESSAGES: &str = "messages";
const NAME: &str = "name";
const SYSTEM: &str = "system";
const TEXT: &str = "text";
const TYPE: &str = "type";

pub fn write_back_system_block(system: &mut Value, block_idx: usize, response: &str) {
    let Value::Array(blocks) = system else {
        return;
    };
    let block = blocks
        .iter_mut()
        .filter(|block| block.get(TYPE).and_then(Value::as_str) == Some(TEXT))
        .nth(block_idx);
    let Some(block) = block else {
        return;
    };
    if let Some(fields) = block.as_object_mut() {
        fields.insert(TEXT.to_owned(), Value::String(response.to_owned()));
    }
}

pub fn write_back_message_text(
    message: &mut Map<String, Value>,
    target: &MessageTextTarget,
    response: &str,
) -> Result<(), WriteBackError> {
    let Some(content) = message.get_mut(CONTENT).filter(|value| !value.is_null()) else {
        return Ok(());
    };
    match target {
        MessageTextTarget::MessageContent { .. } => {
            if let Value::String(slot) = content {
                *slot = response.to_owned();
            }
        }
        MessageTextTarget::ContentBlockText { content_idx, .. } => {
            if let Value::Array(blocks) = content {
                block_at(blocks, *content_idx, target)?
                    .insert(TEXT.to_owned(), Value::String(response.to_owned()));
            }
        }
        MessageTextTarget::ToolResultString { content_idx, .. } => {
            if let Value::Array(blocks) = content {
                block_at(blocks, *content_idx, target)?
                    .insert(CONTENT.to_owned(), Value::String(response.to_owned()));
            }
        }
        MessageTextTarget::ToolResultBlockText {
            content_idx,
            block_idx,
            ..
        } => {
            if let Value::Array(blocks) = content {
                let block = block_at(blocks, *content_idx, target)?;
                let Some(Value::Array(inner)) = block.get_mut(CONTENT) else {
                    return Err(WriteBackError::TargetMissing {
                        target: target.clone(),
                    });
                };
                block_at(inner, *block_idx, target)?
                    .insert(TEXT.to_owned(), Value::String(response.to_owned()));
            }
        }
    }
    Ok(())
}

fn block_at<'blocks>(
    blocks: &'blocks mut [Value],
    index: usize,
    target: &MessageTextTarget,
) -> Result<&'blocks mut Map<String, Value>, WriteBackError> {
    let Some(block) = blocks.get_mut(index) else {
        return Err(WriteBackError::TargetMissing {
            target: target.clone(),
        });
    };
    let Some(fields) = block.as_object_mut() else {
        return Err(WriteBackError::TargetMissing {
            target: target.clone(),
        });
    };
    Ok(fields)
}

#[must_use]
pub fn rewritten_tool_use_input(arguments: &str) -> Option<Map<String, Value>> {
    let parsed: Value = serde_json::from_str(arguments).ok()?;
    match parsed {
        Value::Object(fields) => Some(fields),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            None
        }
    }
}

pub fn write_back_tool_use(
    message: &mut Map<String, Value>,
    target: &ToolUseInputTarget,
    shape: &ToolCallShape,
    rewritten_input: Map<String, Value>,
) {
    let Some(content) = message.get_mut(CONTENT) else {
        return;
    };
    let Value::Array(blocks) = content else {
        return;
    };
    let Some(block) = blocks.get_mut(target.content_idx) else {
        return;
    };
    let Some(fields) = block.as_object_mut() else {
        return;
    };
    fields.insert(INPUT.to_owned(), Value::Object(rewritten_input));
    if let Some(name) = shape.name.as_deref()
        && fields.get(NAME).and_then(Value::as_str) != Some(name)
    {
        fields.insert(NAME.to_owned(), Value::String(name.to_owned()));
    }
}

pub fn apply_tool_calls_to_input(
    messages: &mut [Value],
    scanned_tool_calls: &[ScannedToolCall],
    pre_guardrail_tool_calls: &[ToolCallShape],
    returned_tool_calls: Option<&[Value]>,
    guardrail_name: Option<&str>,
) -> Result<(), ApplyError> {
    let after_guardrail_tool_calls = match returned_tool_calls {
        Some(returned) if returned.len() == pre_guardrail_tool_calls.len() => {
            tool_call_shapes(returned)
        }
        Some(_) | None => tool_call_shapes(
            &scanned_tool_calls
                .iter()
                .map(|scanned| scanned.tool_call.clone())
                .collect::<Vec<Value>>(),
        ),
    };
    let mut rewrites = Vec::with_capacity(scanned_tool_calls.len());
    let pre = pre_guardrail_tool_calls
        .iter()
        .zip(&after_guardrail_tool_calls);
    for (scanned, (before, after)) in scanned_tool_calls.iter().zip(pre) {
        if before == after {
            continue;
        }
        let Some(rewritten_input) = rewritten_tool_use_input(&after.arguments) else {
            return Err(ApplyError::Unappliable(UnappliableRequestRewrite::new(
                guardrail_name,
            )));
        };
        rewrites.push((scanned, after, rewritten_input));
    }
    for (scanned, after, rewritten_input) in rewrites {
        let Some(fields) = messages
            .get_mut(scanned.target.msg_idx)
            .and_then(Value::as_object_mut)
        else {
            return Err(ApplyError::MessageMissing {
                msg_idx: scanned.target.msg_idx,
            });
        };
        write_back_tool_use(fields, &scanned.target, after, rewritten_input);
    }
    Ok(())
}

pub fn apply_responses_to_input(
    data: &mut Map<String, Value>,
    responses: &[String],
    scanned: &[ScannedText],
) -> Result<(), WriteBackError> {
    for (item, response) in scanned.iter().zip(responses) {
        match &item.target {
            InputWriteBackTarget::SystemString => {
                if let Some(system) = data.get_mut(SYSTEM)
                    && system.is_string()
                {
                    *system = Value::String(response.clone());
                }
            }
            InputWriteBackTarget::SystemBlockText { block_idx } => {
                if let Some(system) = data.get_mut(SYSTEM) {
                    write_back_system_block(system, *block_idx, response);
                }
            }
            InputWriteBackTarget::Message(target) => {
                write_message_target(data, target, response)?;
            }
        }
    }
    Ok(())
}

fn write_message_target(
    data: &mut Map<String, Value>,
    target: &MessageTextTarget,
    response: &str,
) -> Result<(), WriteBackError> {
    let message = data
        .get_mut(MESSAGES)
        .and_then(Value::as_array_mut)
        .and_then(|messages| messages.get_mut(message_index(target)));
    let Some(message) = message else {
        return Err(WriteBackError::TargetMissing {
            target: target.clone(),
        });
    };
    let Some(fields) = message.as_object_mut() else {
        return Err(WriteBackError::TargetMissing {
            target: target.clone(),
        });
    };
    write_back_message_text(fields, target, response)
}

const fn message_index(target: &MessageTextTarget) -> usize {
    match target {
        MessageTextTarget::MessageContent { msg_idx }
        | MessageTextTarget::ContentBlockText { msg_idx, .. }
        | MessageTextTarget::ToolResultString { msg_idx, .. }
        | MessageTextTarget::ToolResultBlockText { msg_idx, .. } => *msg_idx,
    }
}
