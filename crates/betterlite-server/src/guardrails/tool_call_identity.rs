#![expect(
    clippy::disallowed_types,
    reason = "guardrail-returned tool calls are provider JSON that betterlite does not model"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

pub mod error;
mod tests;

use std::collections::BTreeSet;

use serde_json::Value;

use error::UndeliverableRewriteReason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCallShape {
    pub name: Option<String>,
    pub arguments: String,
}

#[must_use]
pub fn tool_call_shapes(tool_calls: &[Value]) -> Vec<ToolCallShape> {
    let mut shapes = Vec::with_capacity(tool_calls.len());
    for tool_call in tool_calls {
        shapes.push(tool_call_shape(tool_call));
    }
    shapes
}

fn tool_call_shape(tool_call: &Value) -> ToolCallShape {
    let function = tool_call.get("function").and_then(Value::as_object);
    ToolCallShape {
        name: function
            .and_then(|fields| fields.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        arguments: function
            .and_then(|fields| fields.get("arguments"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    }
}

#[must_use]
pub fn post_guardrail_shapes(
    returned: Option<&[Value]>,
    pre: &[ToolCallShape],
    guardrail_name: Option<&str>,
) -> Vec<ToolCallShape> {
    if pre.is_empty() {
        return pre.to_vec();
    }
    let Some(returned_calls) = returned else {
        warn_about_returned(guardrail_name, "returned no", pre.len());
        return pre.to_vec();
    };
    if returned_calls.len() != pre.len() {
        let count = returned_calls.len().to_string();
        warn_about_returned(guardrail_name, &count, pre.len());
        return pre.to_vec();
    }
    let mut shapes = Vec::with_capacity(returned_calls.len());
    for tool_call in returned_calls {
        let Some(shape) = returned_tool_call_shape(tool_call) else {
            tracing::warn!(
                guardrail = guardrail_name.unwrap_or_default(),
                "OpenAI Responses API: the guardrail returned tool calls without a function name and arguments, leaving the tool call output items unchanged"
            );
            return pre.to_vec();
        };
        shapes.push(shape);
    }
    shapes
}

fn warn_about_returned(guardrail_name: Option<&str>, count: &str, scanned: usize) {
    tracing::warn!(
        guardrail = guardrail_name.unwrap_or_default(),
        returned = count,
        scanned,
        "OpenAI Responses API: the guardrail returned a different number of tool calls than were scanned, leaving the tool call output items unchanged"
    );
}

fn returned_tool_call_shape(tool_call: &Value) -> Option<ToolCallShape> {
    let function = tool_call.as_object()?.get("function")?.as_object()?;
    match function.get("name") {
        None | Some(Value::Null | Value::String(_)) => {}
        Some(_) => return None,
    }
    match function.get("arguments") {
        None | Some(Value::String(_)) => {}
        Some(_) => return None,
    }
    Some(tool_call_shape(tool_call))
}

#[must_use]
pub fn tool_call_rewrite(before: &ToolCallShape, after: &ToolCallShape) -> ToolCallShape {
    ToolCallShape {
        name: if after.name == before.name {
            None
        } else {
            after.name.clone()
        },
        arguments: after.arguments.clone(),
    }
}

pub struct UndeliverableRewriteInput<'calls> {
    pub call_ids: &'calls [String],
    pub tool_call_item_count: usize,
    pub post_guardrail_tool_call_count: usize,
    pub unresolved_argument_event: bool,
    pub rewritten_call_ids: &'calls BTreeSet<String>,
    pub event_call_ids: &'calls BTreeSet<String>,
}

#[must_use]
pub fn undeliverable_reason(
    input: &UndeliverableRewriteInput<'_>,
) -> Option<UndeliverableRewriteReason> {
    if input.call_ids.len() != input.tool_call_item_count {
        return Some(UndeliverableRewriteReason::MissingCallId {
            missing: input
                .tool_call_item_count
                .saturating_sub(input.call_ids.len()),
            total: input.tool_call_item_count,
        });
    }
    let unique: BTreeSet<&String> = input.call_ids.iter().collect();
    if unique.len() != input.call_ids.len() {
        return Some(UndeliverableRewriteReason::RepeatedCallId);
    }
    if input.call_ids.len() != input.post_guardrail_tool_call_count {
        return Some(UndeliverableRewriteReason::CallCountMismatch {
            returned: input.post_guardrail_tool_call_count,
            expected: input.call_ids.len(),
        });
    }
    if input.unresolved_argument_event {
        return Some(UndeliverableRewriteReason::UnresolvedArgumentEvent);
    }
    let missing: Vec<String> = input
        .rewritten_call_ids
        .difference(input.event_call_ids)
        .cloned()
        .collect();
    if missing.is_empty() {
        return None;
    }
    Some(UndeliverableRewriteReason::MissingEventCallId { missing })
}
