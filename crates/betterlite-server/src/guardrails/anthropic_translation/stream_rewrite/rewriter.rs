use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::{Map, Value};

use crate::guardrails::tool_call_identity::ToolCallShape;

// One field of one nested section of a buffered SSE event, rewritten like LiteLLM's _SSEFieldRewrite.
pub struct SseFieldRewrite {
    pub section: &'static str,
    pub field: &'static str,
    pub value: String,
}

// LiteLLM passes a closure; the project forbids closures, so each rewriter is a variant with its own state.
pub enum EventRewriter {
    TextDelta {
        replacements: VecDeque<String>,
    },
    ToolUse {
        rewrites_by_block: BTreeMap<i64, ToolCallShape>,
        arguments_sent: BTreeSet<i64>,
    },
}

impl EventRewriter {
    #[must_use]
    pub fn rewrite(&mut self, event: &Map<String, Value>) -> Option<SseFieldRewrite> {
        match self {
            Self::TextDelta { replacements } => text_delta_rewrite(replacements, event),
            Self::ToolUse {
                rewrites_by_block,
                arguments_sent,
            } => tool_use_rewrite(rewrites_by_block, arguments_sent, event),
        }
    }
}

pub(super) fn is_text_delta(event: &Map<String, Value>) -> bool {
    event.get("type").and_then(Value::as_str) == Some("content_block_delta")
        && event
            .get("delta")
            .and_then(Value::as_object)
            .and_then(|delta| delta.get("type"))
            .and_then(Value::as_str)
            == Some("text_delta")
}

fn text_delta_rewrite(
    replacements: &mut VecDeque<String>,
    event: &Map<String, Value>,
) -> Option<SseFieldRewrite> {
    if !is_text_delta(event) {
        return None;
    }
    Some(SseFieldRewrite {
        section: "delta",
        field: "text",
        value: replacements.pop_front().unwrap_or_default(),
    })
}

fn is_input_json_delta(event: &Map<String, Value>) -> bool {
    event
        .get("delta")
        .and_then(Value::as_object)
        .and_then(|delta| delta.get("type"))
        .and_then(Value::as_str)
        == Some("input_json_delta")
}

fn tool_use_rewrite(
    rewrites_by_block: &BTreeMap<i64, ToolCallShape>,
    arguments_sent: &mut BTreeSet<i64>,
    event: &Map<String, Value>,
) -> Option<SseFieldRewrite> {
    let index = event.get("index").and_then(Value::as_i64)?;
    let shape = rewrites_by_block.get(&index)?;
    match event.get("type").and_then(Value::as_str) {
        Some("content_block_start") => Some(SseFieldRewrite {
            section: "content_block",
            field: "name",
            value: shape.name.clone()?,
        }),
        Some("content_block_delta") => {
            if !is_input_json_delta(event) {
                return None;
            }
            let value = if arguments_sent.insert(index) {
                shape.arguments.clone()
            } else {
                String::new()
            };
            Some(SseFieldRewrite {
                section: "delta",
                field: "partial_json",
                value,
            })
        }
        Some(_) | None => None,
    }
}
