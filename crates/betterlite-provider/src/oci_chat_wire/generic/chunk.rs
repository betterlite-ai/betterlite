use betterlite_openai::stream::chunk::{Choice, Delta, Function, ToolCall};
use betterlite_prompts::arguments::json::Json;

use crate::oci_chat_wire::ids::{normalize_oci_finish_reason, synthesize_oci_tool_call_id};
use crate::oci_chat_wire::parse::parse_chunk;
use crate::oci_chat_wire::types::oci::{OciContentPart, OciStreamChunk, OciToolCall};
use crate::oci_common::OciError;

const IMAGE_STREAM: &str =
    "OCI returned image content in a streaming response, which is not supported";

const TOOL_INDEX: &str = "the OCI stream has more tool calls than a chunk can index";

pub fn handle_generic_stream_chunk(mut chunk: Json) -> Result<Choice, OciError> {
    fill_missing_tool_call_fields(&mut chunk);
    let typed: OciStreamChunk = parse_chunk(&chunk, "OCIStreamChunk")?;
    let text = match &typed.message {
        Some(message) => stream_text(message.content.as_deref())?,
        None => None,
    };
    let tool_calls = match &typed.message {
        Some(message) => match message
            .tool_calls
            .as_ref()
            .filter(|calls| !calls.is_empty())
        {
            Some(calls) => Some(stream_tool_calls(calls)?),
            None => None,
        },
        None => None,
    };
    Ok(Choice {
        index: typed.index.unwrap_or(0),
        delta: Delta {
            role: None,
            content: text,
            refusal: None,
            tool_calls,
            function_call: None,
        },
        finish_reason: normalize_oci_finish_reason(typed.finish_reason.as_deref()),
        logprobs: None,
    })
}

fn stream_text(parts: Option<&[OciContentPart]>) -> Result<Option<String>, OciError> {
    let Some(content_parts) = parts.filter(|given| !given.is_empty()) else {
        return Ok(None);
    };
    let mut text = String::new();
    for part in content_parts {
        match part {
            OciContentPart::Text(content) => text.push_str(&content.text),
            OciContentPart::Image(_) => {
                return Err(OciError {
                    status: 500,
                    message: String::from(IMAGE_STREAM),
                });
            }
            OciContentPart::Other(other) => {
                return Err(OciError {
                    status: 500,
                    message: format!(
                        "Unsupported content type in OCI streaming response: {}",
                        other.kind
                    ),
                });
            }
        }
    }
    Ok(Some(text))
}

fn fill_missing_tool_call_fields(chunk: &mut Json) {
    let Json::Object(chunk_fields) = chunk else {
        return;
    };
    let Some(Json::Object(message)) = chunk_fields.get_mut("message") else {
        return;
    };
    let Some(Json::List(tool_calls)) = message.get_mut("toolCalls") else {
        return;
    };
    for tool_call in tool_calls {
        let Json::Object(call_fields) = tool_call else {
            continue;
        };
        for name in ["id", "name", "arguments"] {
            call_fields
                .entry(String::from(name))
                .or_insert_with(|| Json::Text(String::new()));
        }
    }
}

fn stream_tool_calls(tool_calls: &[OciToolCall]) -> Result<Vec<ToolCall>, OciError> {
    let mut adapted = Vec::with_capacity(tool_calls.len());
    for (position, tool_call) in tool_calls.iter().enumerate() {
        let index = u32::try_from(position).map_err(|_error| OciError {
            status: 500,
            message: String::from(TOOL_INDEX),
        })?;
        adapted.push(ToolCall {
            index,
            id: Some(
                tool_call
                    .id
                    .clone()
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(|| {
                        synthesize_oci_tool_call_id(position, &tool_call.name, &tool_call.arguments)
                    }),
            ),
            kind: Some(String::from("function")),
            function: Some(Function {
                name: Some(tool_call.name.clone()),
                arguments: tool_call.arguments.clone(),
            }),
        });
    }
    Ok(adapted)
}
