use betterlite_openai::stream::chunk::{Choice, Delta, Function, ToolCall};
use betterlite_prompts::arguments::json::Json;

use crate::oci_chat_wire::cohere::tool_calls::adapt_call;
use crate::oci_chat_wire::ids::normalize_oci_finish_reason;
use crate::oci_chat_wire::parse::parse_chunk;
use crate::oci_chat_wire::types::cohere::{CohereStreamChunk, CohereToolCall};
use crate::oci_common::OciError;

const TOOL_INDEX: &str = "the OCI stream has more tool calls than a chunk can index";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CohereEmitted {
    pub tool_calls: bool,
    pub text: bool,
}

pub fn handle_cohere_stream_chunk(
    chunk: &Json,
    emitted: CohereEmitted,
) -> Result<Choice, OciError> {
    let typed: CohereStreamChunk = parse_chunk(chunk, "CohereStreamChunk")?;
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

fn stream_tool_calls(calls: &[CohereToolCall]) -> Result<Vec<ToolCall>, OciError> {
    let mut adapted = Vec::with_capacity(calls.len());
    for (position, call) in calls.iter().enumerate() {
        let index = u32::try_from(position).map_err(|_error| OciError {
            status: 500,
            message: String::from(TOOL_INDEX),
        })?;
        let adapted_call = adapt_call(position, call);
        adapted.push(ToolCall {
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
