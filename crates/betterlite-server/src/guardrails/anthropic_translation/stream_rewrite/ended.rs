use std::collections::{BTreeMap, BTreeSet, VecDeque};

use betterlite_core::guardrails::streaming_rewrite::failure::UndeliverableStreamRewrite;
use serde_json::{Map, Value};

use super::events::rewrite_ended_stream_events;
use super::rewriter::{EventRewriter, is_text_delta};
use crate::guardrails::anthropic_translation::sse_events::{SseChunk, iter_sse_events};
use crate::guardrails::tool_call_identity::ToolCallShape;

// LiteLLM's _write_ended_stream_text_rewrite: the first text delta carries the rewrite and the rest are blanked.
pub fn rewrite_ended_stream_text(
    responses_so_far: &mut [SseChunk],
    rewritten_text: &str,
    guardrail_name: &str,
) -> Result<(), UndeliverableStreamRewrite> {
    if !responses_so_far
        .iter()
        .flat_map(iter_sse_events)
        .any(|event| is_text_delta(&event))
    {
        return Err(UndeliverableStreamRewrite {
            guardrail_name: guardrail_name.to_owned(),
            reason: "the buffered stream carries no text_delta event to land the text rewrite on"
                .to_owned(),
        });
    }
    let mut rewriter = EventRewriter::TextDelta {
        replacements: VecDeque::from([rewritten_text.to_owned()]),
    };
    rewrite_ended_stream_events(responses_so_far, &mut rewriter);
    Ok(())
}

fn is_tool_use_block(event: &Map<String, Value>) -> bool {
    event.get("type").and_then(Value::as_str) == Some("content_block_start")
        && event
            .get("content_block")
            .and_then(Value::as_object)
            .and_then(|block| block.get("type"))
            .and_then(Value::as_str)
            == Some("tool_use")
}

// LiteLLM's _write_ended_stream_tool_call_rewrites: the nth rewritten call lands on the nth tool_use block.
pub fn rewrite_ended_tool_calls(
    responses_so_far: &mut [SseChunk],
    pre_guardrail_tool_calls: &[ToolCallShape],
    post_guardrail_tool_calls: &[ToolCallShape],
    guardrail_name: &str,
) -> Result<(), UndeliverableStreamRewrite> {
    if post_guardrail_tool_calls == pre_guardrail_tool_calls {
        return Ok(());
    }
    let block_indices: Vec<i64> = responses_so_far
        .iter()
        .flat_map(iter_sse_events)
        .filter(is_tool_use_block)
        .filter_map(|event| event.get("index").and_then(Value::as_i64))
        .collect();
    if block_indices.len() != post_guardrail_tool_calls.len() {
        return Err(UndeliverableStreamRewrite {
            guardrail_name: guardrail_name.to_owned(),
            reason: format!(
                "the guardrail returned {} tool calls for a stream that carried {} tool_use blocks",
                post_guardrail_tool_calls.len(),
                block_indices.len()
            ),
        });
    }
    let mut rewrites_by_block = BTreeMap::new();
    for ((index, before), after) in block_indices
        .iter()
        .zip(pre_guardrail_tool_calls)
        .zip(post_guardrail_tool_calls)
    {
        if before != after {
            rewrites_by_block.insert(*index, after.clone());
        }
    }
    let mut rewriter = EventRewriter::ToolUse {
        rewrites_by_block,
        arguments_sent: BTreeSet::new(),
    };
    rewrite_ended_stream_events(responses_so_far, &mut rewriter);
    Ok(())
}
