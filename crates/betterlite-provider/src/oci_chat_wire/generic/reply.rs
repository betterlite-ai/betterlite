use crate::oci_chat_wire::deltas::stream_tool_calls;
use crate::oci_chat_wire::ids::{normalize_oci_finish_reason, synthesize_oci_tool_call_id};
use crate::oci_chat_wire::parse::parse;
use crate::oci_chat_wire::types::oci::{
    OciCompletionResponse, OciContentPart, OciMessage, OciResponseUsage, OciStreamChunk,
    OciToolCall,
};
use crate::oci_common::OciError;
use betterlite_openai::chat::response::{AssistantMessage, ChatResponse, ChatUsage};
use betterlite_openai::chat::{FunctionCall, ToolCall};
use betterlite_openai::stream::chunk::{Choice, Delta};
use betterlite_prompts::arguments::json::Json;
use chrono::DateTime;
use std::collections::BTreeMap;
const NO_CHOICES: &str = "OCI response contained no choices";
const FILL_NO_CHOICES: &str = "the response to fill has no choices";
const TIME_CREATED: &str = "OCI returned an unreadable timeCreated value";
const TIME_EPOCH: &str = "OCI returned a timeCreated before the Unix epoch";
const IMAGE_STREAM: &str =
    "OCI returned image content in a streaming response, which is not supported";
pub fn handle_generic_response(
    body: &Json,
    status: u16,
    response: &mut ChatResponse,
) -> Result<(), OciError> {
    let completion: OciCompletionResponse = parse(
        body,
        status,
        "Response cannot be casted to OCICompletionResponse",
    )?;
    response.created = parse_time_created(&completion.chat_response.time_created, status)?;
    response.model = completion.model_id;
    let Some(response_choice) = completion.chat_response.choices.first() else {
        return Err(OciError {
            status,
            message: String::from(NO_CHOICES),
        });
    };
    let Some(target) = response.choices.first_mut() else {
        return Err(OciError {
            status,
            message: String::from(FILL_NO_CHOICES),
        });
    };
    if let Some(message) = &response_choice.message {
        fill_message(&mut target.message, message);
    }
    target.finish_reason = normalize_oci_finish_reason(response_choice.finish_reason.as_deref());
    response.usage = chat_usage(&completion.chat_response.usage);
    Ok(())
}
pub fn handle_generic_stream_chunk(mut chunk: Json) -> Result<Choice, OciError> {
    fill_missing_tool_call_fields(&mut chunk);
    let typed: OciStreamChunk = parse(&chunk, 500, "Chunk cannot be parsed as OCIStreamChunk")?;
    let text = match &typed.message {
        Some(message) => stream_text(message.content.as_deref())?,
        None => None,
    };
    let tool_calls = typed
        .message
        .as_ref()
        .and_then(|message| message.tool_calls.as_ref())
        .filter(|calls| !calls.is_empty())
        .map(|calls| stream_tool_calls(adapt_tool_calls(calls)));
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
fn parse_time_created(text: &str, status: u16) -> Result<u64, OciError> {
    let parsed = DateTime::parse_from_rfc3339(text).map_err(|_error| OciError {
        status,
        message: String::from(TIME_CREATED),
    })?;
    u64::try_from(parsed.timestamp()).map_err(|_error| OciError {
        status,
        message: String::from(TIME_EPOCH),
    })
}
fn fill_message(target: &mut AssistantMessage, source: &OciMessage) {
    if let Some(parts) = source.content.as_ref().filter(|parts| !parts.is_empty()) {
        let mut text = String::new();
        let mut found = false;
        for part in parts {
            if let OciContentPart::Text(content) = part {
                text.push_str(&content.text);
                found = true;
            }
        }
        if found {
            target.content = Some(text);
        }
    }
    if let Some(tool_calls) = source.tool_calls.as_ref().filter(|calls| !calls.is_empty()) {
        target.tool_calls = Some(adapt_tool_calls(tool_calls));
    }
}
fn tool_call_id(position: usize, tool_call: &OciToolCall) -> String {
    tool_call
        .id
        .clone()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| {
            synthesize_oci_tool_call_id(position, &tool_call.name, &tool_call.arguments)
        })
}
fn adapt_tool_calls(tool_calls: &[OciToolCall]) -> Vec<ToolCall> {
    tool_calls
        .iter()
        .enumerate()
        .map(|(position, tool_call)| ToolCall {
            id: tool_call_id(position, tool_call),
            kind: String::from("function"),
            function: FunctionCall {
                name: tool_call.name.clone(),
                arguments: tool_call.arguments.clone(),
            },
        })
        .collect()
}
fn chat_usage(usage: &OciResponseUsage) -> ChatUsage {
    let completion_tokens = usage.completion_tokens.unwrap_or(0);
    let reasoning_tokens = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|details| details.reasoning_tokens)
        .filter(|tokens| *tokens != 0);
    let completion_tokens_details = reasoning_tokens.map(|tokens| {
        BTreeMap::from([
            (String::from("reasoning_tokens"), Some(tokens)),
            (
                String::from("text_tokens"),
                Some(completion_tokens.saturating_sub(tokens)),
            ),
        ])
    });
    ChatUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: None,
        completion_tokens_details,
    }
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
