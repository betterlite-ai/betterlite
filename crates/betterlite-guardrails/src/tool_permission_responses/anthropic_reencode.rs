use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use crate::guardrail_message_scope::Object;
use crate::tool_permission::types::{mapping_value, object_list, object_mapping};

use super::anthropic_chunks::{count_value, text_field};
use super::error::ResponseToolError;
use super::extraction::id_text;

const THOUGHT_SIGNATURE_SEPARATOR: &str = "__thought__";
const TOOL_USE_ID_FALLBACK: &str = "tool_use_id";
const UNKNOWN_MODEL: &str = "unknown-model";
const ARGUMENTS_CONTEXT: &str = "Anthropic pass-through adapter";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: GuardrailValue,
    },
}

#[derive(Debug, Clone, Serialize)]
struct Usage {
    input_tokens: i64,
    output_tokens: i64,
    #[serde(
        rename = "cache_creation_input_tokens",
        skip_serializing_if = "Option::is_none"
    )]
    cache_creation: Option<i64>,
    #[serde(
        rename = "cache_read_input_tokens",
        skip_serializing_if = "Option::is_none"
    )]
    cache_read: Option<i64>,
}

#[derive(Debug, Serialize)]
struct StartUsage {
    input_tokens: i64,
    output_tokens: i64,
}

#[derive(Debug, Serialize)]
struct MessageStartEvent<'response> {
    #[serde(rename = "type")]
    kind: &'static str,
    message: StartMessage<'response>,
}

#[derive(Debug, Serialize)]
struct StartMessage<'response> {
    id: &'response str,
    #[serde(rename = "type")]
    kind: &'static str,
    role: &'static str,
    model: &'response str,
    content: Vec<ContentBlock>,
    stop_reason: Option<&'static str>,
    stop_sequence: Option<&'static str>,
    usage: StartUsage,
}

#[derive(Debug, Serialize)]
struct BlockStartEvent {
    #[serde(rename = "type")]
    kind: &'static str,
    index: usize,
    content_block: BlockStart,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum BlockStart {
    #[serde(rename = "text")]
    Text { text: &'static str },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: GuardrailValue,
    },
}

#[derive(Debug, Serialize)]
struct BlockDeltaEvent {
    #[serde(rename = "type")]
    kind: &'static str,
    index: usize,
    delta: BlockDelta,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum BlockDelta {
    #[serde(rename = "text_delta")]
    Text { text: String },
    #[serde(rename = "input_json_delta")]
    InputJson { partial_json: String },
}

#[derive(Debug, Serialize)]
struct BlockStopEvent {
    #[serde(rename = "type")]
    kind: &'static str,
    index: usize,
}

#[derive(Debug, Serialize)]
struct MessageDeltaEvent {
    #[serde(rename = "type")]
    kind: &'static str,
    delta: StopDelta,
    usage: Usage,
}

#[derive(Debug, Serialize)]
struct StopDelta {
    stop_reason: &'static str,
    stop_sequence: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct MessageStopEvent {
    #[serde(rename = "type")]
    kind: &'static str,
    usage: Usage,
}

// LiteLLM's anthropic_sse_chunks_from_response: the rewritten response translated back and streamed as its fake iterator does
pub(crate) fn encode_anthropic_stream(
    response: &Object,
) -> Result<Vec<Vec<u8>>, ResponseToolError> {
    let mut frames = vec![start_frame(response)?];
    for (index, block) in content_blocks(response)?.iter().enumerate() {
        frames.extend(block_frames(index, block)?);
    }
    frames.push(delta_frame(response)?);
    frames.push(stop_frame(response)?);
    Ok(frames)
}

fn start_frame(response: &Object) -> Result<Vec<u8>, ResponseToolError> {
    let usage = anthropic_usage(response);
    let message = StartMessage {
        id: text_field(response, "id").unwrap_or(""),
        kind: "message",
        role: "assistant",
        model: model_text(response),
        content: Vec::new(),
        stop_reason: None,
        stop_sequence: None,
        usage: StartUsage {
            input_tokens: usage.input_tokens,
            output_tokens: 0,
        },
    };
    sse_frame(
        "message_start",
        &MessageStartEvent {
            kind: "message_start",
            message,
        },
    )
}

fn delta_frame(response: &Object) -> Result<Vec<u8>, ResponseToolError> {
    sse_frame(
        "message_delta",
        &MessageDeltaEvent {
            kind: "message_delta",
            delta: StopDelta {
                stop_reason: finish_reason(response),
                stop_sequence: None,
            },
            usage: anthropic_usage(response),
        },
    )
}

fn stop_frame(response: &Object) -> Result<Vec<u8>, ResponseToolError> {
    sse_frame(
        "message_stop",
        &MessageStopEvent {
            kind: "message_stop",
            usage: anthropic_usage(response),
        },
    )
}

// Every choice's text and tool calls become blocks, text first, the way _translate_openai_content_to_anthropic reads them
fn content_blocks(response: &Object) -> Result<Vec<ContentBlock>, ResponseToolError> {
    let mut blocks = Vec::new();
    let Some(choices) = response.get("choices").and_then(object_list) else {
        return Ok(blocks);
    };
    for choice in choices {
        let Some(message) = mapping_value(choice, "message") else {
            continue;
        };
        if let Some(GuardrailValue::Text(text)) = mapping_value(message, "content") {
            blocks.push(ContentBlock::Text { text: text.clone() });
        }
        if let Some(tool_calls) = mapping_value(message, "tool_calls").and_then(object_list) {
            for tool_call in tool_calls {
                blocks.push(tool_block(tool_call)?);
            }
        }
    }
    Ok(blocks)
}

fn tool_block(tool_call: &GuardrailValue) -> Result<ContentBlock, ResponseToolError> {
    let function = mapping_value(tool_call, "function");
    let name = match function.and_then(|function| mapping_value(function, "name")) {
        Some(GuardrailValue::Text(name)) => name.clone(),
        None | Some(_) => String::new(),
    };
    let arguments = function.and_then(|function| mapping_value(function, "arguments"));
    let input = parse_arguments(arguments, &name)?;
    Ok(ContentBlock::ToolUse {
        id: normalize_id(id_text(tool_call)),
        name,
        input,
    })
}

// LiteLLM's normalize_anthropic_tool_use_id: strip a thought signature, replace invalid characters, never end empty
fn normalize_id(raw: &str) -> String {
    let base = raw
        .split_once(THOUGHT_SIGNATURE_SEPARATOR)
        .map_or(raw, |(base, _signature)| base);
    let sanitized: String = base
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        return TOOL_USE_ID_FALLBACK.to_owned();
    }
    sanitized
}

// LiteLLM's parse_tool_call_arguments: a blank or absent string is an empty object, and a truncated one is repaired.
fn parse_arguments(
    arguments: Option<&GuardrailValue>,
    tool: &str,
) -> Result<GuardrailValue, ResponseToolError> {
    let Some(GuardrailValue::Text(raw)) = arguments else {
        return Ok(GuardrailValue::Object(Object::new()));
    };
    if raw.trim().is_empty() {
        return Ok(GuardrailValue::Object(Object::new()));
    }
    match serde_json::from_str::<GuardrailValue>(raw) {
        Ok(value) => Ok(value),
        Err(error) => match repair_json(raw) {
            Some(value) => Ok(value),
            None => Err(ResponseToolError::ToolArguments {
                tool: tool.to_owned(),
                context: ARGUMENTS_CONTEXT.to_owned(),
                message: error.to_string(),
                arguments: raw.clone(),
            }),
        },
    }
}

// LiteLLM's _attempt_json_repair: close the openers a truncated stream left behind, in nesting order
fn repair_json(raw: &str) -> Option<GuardrailValue> {
    let stripped = raw.trim_end();
    if stripped.is_empty() {
        return None;
    }
    let mut openers = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for character in stripped.chars() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = in_string;
        } else if character == '"' {
            in_string = !in_string;
        } else if !in_string {
            match character {
                '{' => openers.push('}'),
                '[' => openers.push(']'),
                '}' | ']' => {
                    if openers.last() == Some(&character) {
                        openers.pop();
                    }
                }
                _ => {}
            }
        }
    }
    if openers.is_empty() {
        return None;
    }
    let mut candidate = stripped.trim_end_matches(',').to_owned();
    for closer in openers.iter().rev() {
        candidate.push(*closer);
    }
    serde_json::from_str(&candidate).ok()
}

fn anthropic_usage(response: &Object) -> Usage {
    let fields = response.get("usage").and_then(object_mapping);
    let prompt = count_in(fields, "prompt_tokens");
    let completion = count_in(fields, "completion_tokens");
    let cache_creation = count_in(fields, "cache_creation_input_tokens");
    let cache_read = count_in(fields, "cache_read_input_tokens");
    let input = prompt
        .saturating_sub(cache_read)
        .saturating_sub(cache_creation)
        .max(0);
    Usage {
        input_tokens: input,
        output_tokens: completion,
        cache_creation: (cache_creation > 0).then_some(cache_creation),
        cache_read: (cache_read > 0).then_some(cache_read),
    }
}

fn count_in(fields: Option<&Object>, key: &str) -> i64 {
    match fields.and_then(|fields| fields.get(key)) {
        Some(value) => count_value(value),
        None => 0,
    }
}

fn block_frames(index: usize, block: &ContentBlock) -> Result<Vec<Vec<u8>>, ResponseToolError> {
    Ok(vec![
        sse_frame(
            "content_block_start",
            &BlockStartEvent {
                kind: "content_block_start",
                index,
                content_block: block_start(block),
            },
        )?,
        sse_frame(
            "content_block_delta",
            &BlockDeltaEvent {
                kind: "content_block_delta",
                index,
                delta: block_delta(block)?,
            },
        )?,
        sse_frame(
            "content_block_stop",
            &BlockStopEvent {
                kind: "content_block_stop",
                index,
            },
        )?,
    ])
}

fn block_start(block: &ContentBlock) -> BlockStart {
    match block {
        ContentBlock::Text { .. } => BlockStart::Text { text: "" },
        ContentBlock::ToolUse { id, name, .. } => BlockStart::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: GuardrailValue::Object(Object::new()),
        },
    }
}

fn block_delta(block: &ContentBlock) -> Result<BlockDelta, ResponseToolError> {
    match block {
        ContentBlock::Text { text } => Ok(BlockDelta::Text { text: text.clone() }),
        ContentBlock::ToolUse { input, .. } => Ok(BlockDelta::InputJson {
            partial_json: serde_json::to_string(input)?,
        }),
    }
}

fn sse_frame(kind: &str, event: &impl Serialize) -> Result<Vec<u8>, ResponseToolError> {
    let data = serde_json::to_string(event)?;
    Ok(format!("event: {kind}\ndata: {data}\n\n").into_bytes())
}

fn model_text(response: &Object) -> &str {
    match text_field(response, "model") {
        Some(model) if !model.is_empty() => model,
        None | Some(_) => UNKNOWN_MODEL,
    }
}

fn finish_reason(response: &Object) -> &'static str {
    let choice = response
        .get("choices")
        .and_then(object_list)
        .and_then(|choices| choices.first());
    match choice.and_then(|choice| mapping_value(choice, "finish_reason")) {
        Some(GuardrailValue::Text(reason)) => translated_finish_reason(reason),
        None | Some(_) => "end_turn",
    }
}

// LiteLLM's _translate_openai_finish_reason_to_anthropic
fn translated_finish_reason(reason: &str) -> &'static str {
    match reason {
        "stop" => "end_turn",
        "length" => "max_tokens",
        "tool_calls" => "tool_use",
        "content_filter" | "refusal" => "refusal",
        _ => "end_turn",
    }
}
