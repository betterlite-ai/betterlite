use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::gemini_tool_invokes::invoke::convert_tool_calls_to_gemini;
use betterlite_prompts::gemini_tool_invokes::types::GeminiTool;
use betterlite_prompts::gemini_tool_results::types::LastToolCallMessage;
use betterlite_prompts::media::limits::MAX_MESSAGE_PARTS;
use betterlite_prompts::message_content::{Content, Message};

use crate::gemini_history::content::{
    MediaInput, Part, ServerToolCall, ServerToolCallPart, ServerToolResponse,
    ServerToolResponsePart, TextPart, ThoughtPart, part_object, push_media,
};
use crate::gemini_history::error::HistoryError;
use crate::gemini_history::fields::{DETAIL, mime_hints, optional_string, required_string};
use crate::gemini_history::signatures::collect_tool_signatures;
use crate::gemini_history::thinking::decode_thinking_part;
use crate::gemini_media::equivalence::part_exists;
use crate::gemini_media::error::MediaField;
use crate::gemini_media::resolution::convert_detail;

const TEXT: &str = "text";
const TYPE: &str = "type";
const THINKING: &str = "thinking";
const THINKING_BLOCKS: &str = "thinking_blocks";
const THINKING_KIND: &str = "thinking";
const SIGNATURE: &str = "signature";
const REASONING_CONTENT: &str = "reasoning_content";
const IMAGES: &str = "images";
const IMAGE_URL: &str = "image_url";
const URL: &str = "url";
const TOOL_CALLS: &str = "tool_calls";
const FUNCTION_CALL: &str = "function_call";
const PROVIDER_FIELDS: &str = "provider_specific_fields";
const THOUGHT_SIGNATURES: &str = "thought_signatures";
const SERVER_INVOCATIONS: &str = "server_side_tool_invocations";
const TOOL_TYPE: &str = "tool_type";
const ID: &str = "id";
const ARGS: &str = "args";
const RESPONSE: &str = "response";
const THOUGHT_SIGNATURE: &str = "thought_signature";
const RESPONSE_THOUGHT_SIGNATURE: &str = "response_thought_signature";
const EXCLUDED_KEYS: [&str; 1] = ["thoughtSignature"];

#[derive(Clone, Copy)]
pub struct AssistantOptions<'model> {
    pub model: Option<&'model str>,
    pub forward_function_call_id: bool,
}

pub fn append_assistant_parts(
    message: &Message,
    index: usize,
    options: AssistantOptions<'_>,
    parts: &mut Vec<Part>,
    last_tool_calls: &mut Option<LastToolCallMessage>,
) -> Result<(), HistoryError> {
    append_reasoning(message, index, parts)?;
    append_thinking_blocks(message, index, parts)?;
    append_content(message, index, parts)?;
    append_generated_images(message, index, options.model, parts)?;
    append_tool_calls(message, options, parts, last_tool_calls)?;
    append_server_invocations(message, index, parts)?;
    Ok(())
}

fn append_reasoning(
    message: &Message,
    index: usize,
    parts: &mut Vec<Part>,
) -> Result<(), HistoryError> {
    let path = format!("messages[{index}].reasoning_content");
    if let Some(text) =
        optional_string(&message.provider_specific_fields, REASONING_CONTENT, &path)?
    {
        parts.push(Part::Thought(ThoughtPart {
            thought: true,
            text,
        }));
    }
    Ok(())
}

fn append_thinking_blocks(
    message: &Message,
    index: usize,
    parts: &mut Vec<Part>,
) -> Result<(), HistoryError> {
    let path = format!("messages[{index}].thinking_blocks");
    match message.provider_specific_fields.get(THINKING_BLOCKS) {
        None | Some(Json::Null) => Ok(()),
        Some(Json::List(blocks)) => {
            for (block_index, block) in blocks.iter().enumerate() {
                append_thinking_block(block, &format!("{path}[{block_index}]"), parts)?;
            }
            Ok(())
        }
        Some(_) => Err(HistoryError::WrongField {
            path,
            expected: "a list",
        }),
    }
}

fn append_thinking_block(
    block: &Json,
    path: &str,
    parts: &mut Vec<Part>,
) -> Result<(), HistoryError> {
    let Json::Object(fields) = block else {
        return Err(HistoryError::WrongField {
            path: path.to_owned(),
            expected: "an object",
        });
    };
    let kind = required_string(fields, TYPE, path)?;
    if kind != THINKING_KIND {
        return Ok(());
    }
    let Some(thinking) = optional_string(fields, THINKING, path)? else {
        return Ok(());
    };
    let Some(signature) = optional_string(fields, SIGNATURE, path)? else {
        return Ok(());
    };
    parts.push(decode_thinking_part(&thinking, &signature));
    Ok(())
}

fn append_content(
    message: &Message,
    index: usize,
    parts: &mut Vec<Part>,
) -> Result<(), HistoryError> {
    match &message.content {
        None => Ok(()),
        Some(Content::Text(text)) => {
            parts.push(assistant_text_part(message, index, text.clone())?);
            Ok(())
        }
        Some(Content::Parts(items)) => {
            if items.len() > MAX_MESSAGE_PARTS {
                return Err(HistoryError::TooManyParts {
                    index,
                    limit: MAX_MESSAGE_PARTS,
                });
            }
            for (part_index, item) in items.iter().enumerate() {
                if item.kind.as_str() != TEXT {
                    continue;
                }
                let text = required_string(
                    &item.fields,
                    TEXT,
                    &format!("messages[{index}].content[{part_index}]"),
                )?;
                parts.push(Part::Text(TextPart {
                    text,
                    signature: None,
                }));
            }
            Ok(())
        }
    }
}

fn assistant_text_part(
    message: &Message,
    index: usize,
    text: String,
) -> Result<Part, HistoryError> {
    let tool_signatures = collect_tool_signatures(message);
    let signature = text_signature(message);
    if matches!(&signature, Some(Json::List(_) | Json::Object(_))) {
        return Err(HistoryError::WrongField {
            path: format!("messages[{index}].provider_specific_fields.thought_signatures[0]"),
            expected: "a signature string",
        });
    }
    let replay = signature.filter(|value| match value {
        Json::Text(raw) => !tool_signatures.contains(raw),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => true,
    });
    Ok(Part::Text(TextPart {
        text,
        signature: replay,
    }))
}

fn text_signature(message: &Message) -> Option<Json> {
    let provider_fields = message
        .provider_specific_fields
        .get(PROVIDER_FIELDS)?
        .as_object()?;
    match provider_fields.get(THOUGHT_SIGNATURES)? {
        Json::List(signatures) => signatures.first().cloned(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

fn append_generated_images(
    message: &Message,
    index: usize,
    model: Option<&str>,
    parts: &mut Vec<Part>,
) -> Result<(), HistoryError> {
    let path = format!("messages[{index}].images");
    let Some(Json::List(items)) = message.provider_specific_fields.get(IMAGES) else {
        return Ok(());
    };
    for (image_index, item) in items.iter().enumerate() {
        append_generated_image(item, &format!("{path}[{image_index}]"), model, parts)?;
    }
    Ok(())
}

fn append_generated_image(
    item: &Json,
    path: &str,
    model: Option<&str>,
    parts: &mut Vec<Part>,
) -> Result<(), HistoryError> {
    let Some(fields) = item.as_object() else {
        return Ok(());
    };
    let Some(Json::Object(image)) = fields.get(IMAGE_URL) else {
        return Ok(());
    };
    let image_path = format!("{path}.image_url");
    let Some(Json::Text(url)) = image.get(URL) else {
        return match image.get(URL) {
            None | Some(Json::Null) => Ok(()),
            Some(_) => Err(HistoryError::WrongField {
                path: format!("{image_path}.url"),
                expected: "a string",
            }),
        };
    };
    if url.is_empty() {
        return Ok(());
    }
    let hints = mime_hints(image, &image_path)?;
    let resolution = optional_string(image, DETAIL, &image_path)?
        .as_deref()
        .and_then(convert_detail);
    let media = MediaInput {
        url,
        hints: &hints,
        resolution,
        video_metadata: None,
        model,
        field: MediaField::ImageUrl,
    };
    push_media(parts, &media)
}

fn append_tool_calls(
    message: &Message,
    options: AssistantOptions<'_>,
    parts: &mut Vec<Part>,
    last_tool_calls: &mut Option<LastToolCallMessage>,
) -> Result<(), HistoryError> {
    let tool_calls = message.provider_specific_fields.get(TOOL_CALLS);
    let function_call = message.provider_specific_fields.get(FUNCTION_CALL);
    let function_present = function_call.is_some_and(|value| !matches!(value, Json::Null));
    if matches!(tool_calls, Some(Json::Null)) && !function_present {
        return Ok(());
    }
    let converted =
        convert_tool_calls_to_gemini(message, options.model, options.forward_function_call_id)?;
    for part in converted {
        let candidate = part_object(&Part::FunctionCall(part.clone()))?;
        let existing: Vec<Object> = parts
            .iter()
            .map(part_object)
            .collect::<Result<Vec<_>, HistoryError>>()?;
        if part_exists(&existing, &candidate, &EXCLUDED_KEYS) {
            continue;
        }
        parts.push(Part::FunctionCall(part));
    }
    let carries_calls = matches!(tool_calls, Some(Json::List(items)) if !items.is_empty());
    if carries_calls || function_present {
        *last_tool_calls = Some(last_tool_call_message(message)?);
    }
    Ok(())
}

fn last_tool_call_message(message: &Message) -> Result<LastToolCallMessage, HistoryError> {
    let Some(value) = message.provider_specific_fields.get(TOOL_CALLS) else {
        return Ok(LastToolCallMessage { tool_calls: None });
    };
    if matches!(value, Json::Null) {
        return Ok(LastToolCallMessage { tool_calls: None });
    }
    let encoded = serde_json::to_value(value).map_err(|source| HistoryError::Encode { source })?;
    let tool_calls = serde_json::from_value::<Vec<GeminiTool>>(encoded)
        .map_err(|source| HistoryError::ToolCalls { source })?;
    Ok(LastToolCallMessage {
        tool_calls: Some(tool_calls),
    })
}

fn append_server_invocations(
    message: &Message,
    index: usize,
    parts: &mut Vec<Part>,
) -> Result<(), HistoryError> {
    let Some(Json::Object(provider_fields)) = message.provider_specific_fields.get(PROVIDER_FIELDS)
    else {
        return Ok(());
    };
    let Some(Json::List(invocations)) = provider_fields.get(SERVER_INVOCATIONS) else {
        return Ok(());
    };
    for (invocation_index, invocation) in invocations.iter().enumerate() {
        let Some(fields) = invocation.as_object() else {
            return Err(HistoryError::WrongField {
                path: format!(
                    "messages[{index}].provider_specific_fields.server_side_tool_invocations[{invocation_index}]"
                ),
                expected: "an object",
            });
        };
        let tool_type = fields.get(TOOL_TYPE).cloned().unwrap_or(Json::Null);
        parts.push(Part::ServerToolCall(ServerToolCallPart {
            tool_call: ServerToolCall {
                tool_type: tool_type.clone(),
                id: fields.get(ID).cloned().unwrap_or(Json::Null),
                args: fields.get(ARGS).cloned().unwrap_or(Json::Null),
            },
            signature: fields.get(THOUGHT_SIGNATURE).cloned(),
        }));
        let Some(response) = fields.get(RESPONSE) else {
            continue;
        };
        parts.push(Part::ServerToolResponse(ServerToolResponsePart {
            tool_response: ServerToolResponse {
                id: fields.get(ID).cloned().unwrap_or(Json::Null),
                response: response.clone(),
                tool_type: (!matches!(tool_type, Json::Null)).then_some(tool_type),
            },
            signature: fields.get(RESPONSE_THOUGHT_SIGNATURE).cloned(),
        }));
    }
    Ok(())
}
