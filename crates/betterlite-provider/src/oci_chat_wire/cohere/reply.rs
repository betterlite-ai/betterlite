use std::collections::BTreeMap;

use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_openai::chat::{FunctionCall, ToolCall};
use betterlite_openai::stream::chunk::ToolCall as StreamToolCall;
use betterlite_openai::stream::chunk::{Choice, Delta, Function};
use betterlite_prompts::arguments::json::Json;

use crate::oci_chat_wire::ids::{normalize_oci_finish_reason, synthesize_oci_tool_call_id};
use crate::oci_chat_wire::parse::parse;
use crate::oci_chat_wire::types::cohere::{CohereChatResult, CohereStreamChunk, CohereToolCall};
use crate::oci_common::OciError;

const TOOL_INDEX: &str = "the OCI stream has more tool calls than a chunk can index";

const CAST_PREFIX: &str = "Response cannot be casted to CohereChatResult";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CohereEmitted {
    pub tool_calls: bool,
    pub text: bool,
}

pub fn handle_cohere_response(
    body: &Json,
    model: &str,
    created: u64,
    status: u16,
    response: &mut ChatResponse,
) -> Result<(), OciError> {
    let result: CohereChatResult = parse(body, status, CAST_PREFIX)?;
    model.clone_into(&mut response.model);
    response.created = created;
    let text = result.chat_response.text;
    let finish_reason = normalize_oci_finish_reason(result.chat_response.finish_reason.as_deref());
    let tool_calls = result
        .chat_response
        .tool_calls
        .as_ref()
        .filter(|calls| !calls.is_empty())
        .map(|calls| adapt_tool_calls(calls));
    response.choices = vec![ChatChoice {
        index: 0,
        message: AssistantMessage {
            role: String::from("assistant"),
            content: (!text.is_empty()).then_some(text),
            tool_calls,
            annotations: None,
            audio: None,
            refusal: None,
            provider_specific_fields: MessageProviderFields::default(),
        },
        finish_reason,
        logprobs: None,
        provider_specific_fields: BTreeMap::new(),
    }];
    response.usage = match result.chat_response.usage {
        Some(usage) => ChatUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        None => ChatUsage::default(),
    };
    Ok(())
}

pub fn handle_cohere_stream_chunk(
    chunk: &Json,
    emitted: CohereEmitted,
) -> Result<Choice, OciError> {
    let typed: CohereStreamChunk =
        parse(chunk, 500, "Chunk cannot be parsed as CohereStreamChunk")?;
    let restates_text = typed.chat_history.is_some() || typed.tool_calls.is_some();
    let restates_tool_calls = typed.chat_history.is_some();
    let text = if restates_text && emitted.text {
        None
    } else {
        typed.text
    };
    let remaining_calls = if restates_tool_calls && emitted.tool_calls {
        None
    } else {
        typed.tool_calls.as_deref()
    };
    let tool_calls = match remaining_calls.filter(|calls| !calls.is_empty()) {
        Some(calls) => Some(stream_tool_calls(calls)?),
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

struct AdaptedCall {
    id: String,
    name: String,
    arguments: String,
}

fn adapt_tool_calls(calls: &[CohereToolCall]) -> Vec<ToolCall> {
    calls
        .iter()
        .enumerate()
        .map(|(position, call)| {
            let adapted = adapt_call(position, call);
            ToolCall {
                id: adapted.id,
                kind: String::from("function"),
                function: FunctionCall {
                    name: adapted.name,
                    arguments: adapted.arguments,
                },
            }
        })
        .collect()
}

fn stream_tool_calls(calls: &[CohereToolCall]) -> Result<Vec<StreamToolCall>, OciError> {
    let mut adapted = Vec::with_capacity(calls.len());
    for (position, call) in calls.iter().enumerate() {
        let index = u32::try_from(position).map_err(|_error| OciError {
            status: 500,
            message: String::from(TOOL_INDEX),
        })?;
        let adapted_call = adapt_call(position, call);
        adapted.push(StreamToolCall {
            index,
            id: Some(adapted_call.id),
            kind: Some(String::from("function")),
            function: Some(Function {
                name: Some(adapted_call.name),
                arguments: adapted_call.arguments,
            }),
        });
    }
    Ok(adapted)
}

fn adapt_call(position: usize, call: &CohereToolCall) -> AdaptedCall {
    let arguments = serde_json::to_string(&call.parameters).unwrap_or_default();
    let sorted: BTreeMap<&String, &Json> = call.parameters.iter().collect();
    let sorted_text = serde_json::to_string(&sorted).unwrap_or_default();
    AdaptedCall {
        id: synthesize_oci_tool_call_id(position, &call.name, &sorted_text),
        name: call.name.clone(),
        arguments,
    }
}
