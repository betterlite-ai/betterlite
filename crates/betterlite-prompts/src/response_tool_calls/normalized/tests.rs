#![cfg(test)]

use super::NormalizedToolCall;
use crate::response_tool_calls::projections::ToolCallResponse;

fn response(json: &str) -> ToolCallResponse {
    serde_json::from_str(json).expect("response projection")
}

fn calls(json: &str) -> Vec<NormalizedToolCall> {
    serde_json::from_str(json).expect("normalized tool calls")
}

mod anthropic;
mod arguments;
mod chat;
mod dispatch;
mod identity;
mod responses;
