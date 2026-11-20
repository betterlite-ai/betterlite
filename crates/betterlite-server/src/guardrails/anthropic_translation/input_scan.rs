#![expect(
    clippy::disallowed_types,
    reason = "Anthropic request messages and content blocks are provider JSON that betterlite does not model"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

pub mod error;
pub mod images;
mod tests;
pub(crate) mod tool_calls;

use betterlite_guardrails::guardrail_message_scope::scoped_indices::{ScanScope, ScopeFlags};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::guardrails::anthropic_translation::scan_targets::{
    ExtractedInput, InputWriteBackTarget, MessageTextTarget, ScannedText, ScannedToolCall,
    ToolUseInputTarget,
};
use crate::guardrails::anthropic_translation::structured_write_back::translate_to_openai;

use error::{ConvertError, StructuredMessagesError};

const CONTENT: &str = "content";
const IMAGE: &str = "image";
const MESSAGES: &str = "messages";
const ROLE: &str = "role";
const SYSTEM: &str = "system";
const TEXT: &str = "text";
const TOOL: &str = "tool";
const TOOL_RESULT: &str = "tool_result";
const TYPE: &str = "type";

// The one place this module and its siblings move between provider JSON and the typed values around it
pub(crate) fn serde_convert<Source, Target>(
    value: &Source,
    target: &'static str,
) -> Result<Target, ConvertError>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    let json =
        serde_json::to_value(value).map_err(|source| ConvertError::Convert { target, source })?;
    serde_json::from_value(json).map_err(|source| ConvertError::Convert { target, source })
}

#[must_use]
pub fn extract_input_text_and_images(
    message: &Value,
    msg_idx: usize,
    flags: ScopeFlags,
) -> ExtractedInput {
    let role = message
        .get(ROLE)
        .and_then(Value::as_str)
        .unwrap_or_default();
    if role == SYSTEM {
        return if matches!(flags.scan, ScanScope::ToolResultsOnly) {
            ExtractedInput::empty()
        } else {
            extract_midturn_system_text(message, msg_idx)
        };
    }
    if flags.skip_tool_message && role.eq_ignore_ascii_case(TOOL) {
        return ExtractedInput::empty();
    }
    match message.get(CONTENT) {
        Some(Value::String(text)) => {
            if matches!(flags.scan, ScanScope::ToolResultsOnly) {
                return ExtractedInput::empty();
            }
            message_text(text, MessageTextTarget::MessageContent { msg_idx })
        }
        Some(Value::Array(items)) => extract_blocks(items, msg_idx, flags),
        Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_)) | None => {
            ExtractedInput::empty()
        }
    }
}

fn message_text(text: &str, target: MessageTextTarget) -> ExtractedInput {
    ExtractedInput {
        scanned: vec![ScannedText {
            text: text.to_owned(),
            target: InputWriteBackTarget::Message(target),
        }],
        images: Vec::new(),
        tool_calls: Vec::new(),
    }
}

fn extract_blocks(items: &[Value], msg_idx: usize, flags: ScopeFlags) -> ExtractedInput {
    let mut extracted = ExtractedInput::empty();
    for (content_idx, item) in items.iter().enumerate() {
        let Some(block) = item.as_object() else {
            continue;
        };
        let one_block = extract_content_block(block, msg_idx, content_idx, flags);
        extracted.scanned.extend(one_block.scanned);
        extracted.images.extend(one_block.images);
    }
    if !matches!(flags.scan, ScanScope::ToolResultsOnly) {
        for (position, candidate) in items.iter().enumerate() {
            let Some(tool_block) = candidate.as_object() else {
                continue;
            };
            // The running count numbers tool calls among tool_use blocks only, not content positions.
            let Some(tool_call) =
                tool_calls::client_tool_use_to_openai(tool_block, extracted.tool_calls.len())
            else {
                continue;
            };
            extracted.tool_calls.push(ScannedToolCall {
                tool_call,
                target: ToolUseInputTarget {
                    msg_idx,
                    content_idx: position,
                },
            });
        }
    }
    extracted
}

fn extract_content_block(
    block: &Map<String, Value>,
    msg_idx: usize,
    content_idx: usize,
    flags: ScopeFlags,
) -> ExtractedInput {
    if block.get(TYPE).and_then(Value::as_str) == Some(TOOL_RESULT) {
        return if flags.skip_tool_message {
            ExtractedInput::empty()
        } else {
            extract_tool_result(block, msg_idx, content_idx)
        };
    }
    if matches!(flags.scan, ScanScope::ToolResultsOnly) {
        return ExtractedInput::empty();
    }
    let mut extracted = ExtractedInput::empty();
    if let Some(Value::String(text)) = block.get(TEXT) {
        extracted.scanned.push(ScannedText {
            text: text.clone(),
            target: InputWriteBackTarget::Message(MessageTextTarget::ContentBlockText {
                msg_idx,
                content_idx,
            }),
        });
    }
    if block.get(TYPE).and_then(Value::as_str) == Some(IMAGE) {
        extracted.images = images::image_sources(block);
    }
    extracted
}

fn extract_tool_result(
    block: &Map<String, Value>,
    msg_idx: usize,
    content_idx: usize,
) -> ExtractedInput {
    match block.get(CONTENT) {
        Some(Value::String(text)) => message_text(
            text,
            MessageTextTarget::ToolResultString {
                msg_idx,
                content_idx,
            },
        ),
        Some(Value::Array(items)) => {
            let mut extracted = ExtractedInput::empty();
            for (block_idx, item) in items.iter().enumerate() {
                let Some(fields) = item.as_object() else {
                    continue;
                };
                if let Some(Value::String(text)) = fields.get(TEXT) {
                    extracted.scanned.push(ScannedText {
                        text: text.clone(),
                        target: InputWriteBackTarget::Message(
                            MessageTextTarget::ToolResultBlockText {
                                msg_idx,
                                content_idx,
                                block_idx,
                            },
                        ),
                    });
                }
                if fields.get(TYPE).and_then(Value::as_str) == Some(IMAGE) {
                    extracted.images.extend(images::image_sources(fields));
                }
            }
            extracted
        }
        Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_)) | None => {
            ExtractedInput::empty()
        }
    }
}

fn midturn_text_block(item: &Value, msg_idx: usize, content_idx: usize) -> Option<ScannedText> {
    let fields = item.as_object()?;
    if fields.get(TYPE).and_then(Value::as_str) != Some(TEXT) {
        return None;
    }
    let text = fields
        .get(TEXT)?
        .as_str()
        .filter(|block_text| !block_text.is_empty())?;
    Some(ScannedText {
        text: text.to_owned(),
        target: InputWriteBackTarget::Message(MessageTextTarget::ContentBlockText {
            msg_idx,
            content_idx,
        }),
    })
}

fn extract_midturn_system_text(message: &Value, msg_idx: usize) -> ExtractedInput {
    let Some(content) = message.get(CONTENT) else {
        return ExtractedInput::empty();
    };
    match content {
        Value::String(text) if !text.is_empty() => {
            message_text(text, MessageTextTarget::MessageContent { msg_idx })
        }
        Value::Array(items) => ExtractedInput {
            scanned: items
                .iter()
                .enumerate()
                .filter_map(|(content_idx, item)| midturn_text_block(item, msg_idx, content_idx))
                .collect(),
            images: Vec::new(),
            tool_calls: Vec::new(),
        },
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) | Value::String(_) => {
            ExtractedInput::empty()
        }
    }
}

#[must_use]
pub fn extract_top_level_system_text(hoisted: &Value) -> Vec<ScannedText> {
    match hoisted.get(CONTENT) {
        Some(Value::String(text)) => vec![ScannedText {
            text: text.clone(),
            target: InputWriteBackTarget::SystemString,
        }],
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .filter_map(|(block_idx, item)| {
                let block_text = item.as_object()?.get(TEXT)?.as_str()?;
                Some(ScannedText {
                    text: block_text.to_owned(),
                    target: InputWriteBackTarget::SystemBlockText { block_idx },
                })
            })
            .collect(),
        Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_)) | None => {
            Vec::new()
        }
    }
}

pub async fn get_structured_messages(
    data: &Map<String, Value>,
) -> Result<Option<Vec<Value>>, StructuredMessagesError> {
    if data.get(MESSAGES).is_none_or(Value::is_null) {
        return Ok(None);
    }
    let translated = translate_to_openai(data).await?;
    let encoded = serde_json::to_value(&translated)?;
    let messages = encoded
        .get(MESSAGES)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok((!messages.is_empty()).then_some(messages))
}
