use std::collections::BTreeMap;

use betterlite_openai::chat::response::{AssistantMessage, ChatResponse, ChatUsage};
use betterlite_openai::chat::{FunctionCall, ToolCall};
use betterlite_prompts::arguments::json::Json;
use chrono::DateTime;

use crate::oci_chat_wire::ids::{normalize_oci_finish_reason, synthesize_oci_tool_call_id};
use crate::oci_chat_wire::parse::parse_response;
use crate::oci_chat_wire::types::oci::{
    OciCompletionResponse, OciContentPart, OciMessage, OciResponseChoice, OciResponseUsage,
    OciToolCall,
};
use crate::oci_common::OciError;

const NO_CHOICES: &str = "OCI response contained no choices";

const FILL_NO_CHOICES: &str = "the response to fill has no choices";

const TIME_CREATED: &str = "OCI returned an unreadable timeCreated value";

const TIME_EPOCH: &str = "OCI returned a timeCreated before the Unix epoch";

pub fn handle_generic_response(
    body: &Json,
    status: u16,
    response: &mut ChatResponse,
) -> Result<(), OciError> {
    let completion: OciCompletionResponse = parse_response(body, "OCICompletionResponse", status)?;
    response.created = parse_time_created(&completion.chat_response.time_created, status)?;
    response.model = completion.model_id;
    let Some(response_choice) = completion.chat_response.choices.first() else {
        return Err(OciError {
            status,
            message: String::from(NO_CHOICES),
        });
    };
    reject_unknown_parts(&completion.chat_response.choices, status)?;
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

fn reject_unknown_parts(choices: &[OciResponseChoice], status: u16) -> Result<(), OciError> {
    for choice in choices {
        let Some(message) = &choice.message else {
            continue;
        };
        let Some(parts) = &message.content else {
            continue;
        };
        for part in parts {
            if let OciContentPart::Other(other) = part {
                return Err(OciError {
                    status,
                    message: format!(
                        "Response cannot be casted to OCICompletionResponse: unsupported content type `{}`",
                        other.kind
                    ),
                });
            }
        }
    }
    Ok(())
}

fn adapt_tool_calls(tool_calls: &[OciToolCall]) -> Vec<ToolCall> {
    tool_calls
        .iter()
        .enumerate()
        .map(|(position, tool_call)| ToolCall {
            id: tool_call
                .id
                .clone()
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| {
                    synthesize_oci_tool_call_id(position, &tool_call.name, &tool_call.arguments)
                }),
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
