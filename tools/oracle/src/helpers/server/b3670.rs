use std::collections::BTreeSet;

use betterlite_server::guardrails::anthropic_translation::scan_targets::is_client_tool_use;
use betterlite_server::guardrails::responses_translation::input_provenance::{
    InputProvenance, input_item_provenance, item_rewrite_field, rewritten_input_item,
};
use betterlite_server::guardrails::tool_call_identity::{
    ToolCallShape, UndeliverableRewriteInput, post_guardrail_shapes, tool_call_rewrite,
    tool_call_shapes, undeliverable_reason,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Request {
    InputItemProvenance {
        raw_input: Vec<Value>,
        expected_messages: Vec<Value>,
    },
    ItemRewriteField {
        item: Map<String, Value>,
    },
    RewrittenItem {
        item: Map<String, Value>,
        rewritten: Value,
    },
    ToolCallShapes {
        calls: Vec<Value>,
    },
    PostGuardrailShapes {
        returned: Option<Vec<Value>>,
        pre: Vec<Value>,
        guardrail_name: Option<String>,
    },
    ToolCallRewrite {
        before: Value,
        after: Value,
    },
    UndeliverableReason {
        call_ids: Vec<String>,
        tool_call_item_count: usize,
        post_guardrail_tool_call_count: usize,
        unresolved_argument_event: bool,
        rewritten_call_ids: BTreeSet<String>,
        event_call_ids: BTreeSet<String>,
    },
    ClientToolUse {
        block: Map<String, Value>,
    },
}

impl Helper for Op {
    type Input = Request;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(match input {
            Request::InputItemProvenance {
                raw_input,
                expected_messages,
            } => provenance_answer(raw_input, expected_messages)?,
            Request::ItemRewriteField { item } => item_rewrite_field(item)
                .map_or(Value::Null, |field| Value::String(field.to_owned())),
            Request::RewrittenItem { item, rewritten } => rewritten_item_answer(item, rewritten)?,
            Request::ToolCallShapes { calls } => shapes_value(&tool_call_shapes(calls)),
            Request::PostGuardrailShapes {
                returned,
                pre,
                guardrail_name,
            } => post_shapes_answer(returned.as_deref(), pre, guardrail_name.as_deref()),
            Request::ToolCallRewrite { before, after } => rewrite_answer(before, after),
            Request::UndeliverableReason { .. } => undeliverable_answer(input),
            Request::ClientToolUse { block } => Value::Bool(is_client_tool_use(block)),
        })
    }
}

fn provenance_answer(raw_input: &[Value], expected_messages: &[Value]) -> HelperResult {
    input_item_provenance(raw_input, expected_messages)
        .or_value_error()
        .map(|provenance| provenance_value(provenance.as_ref()))
}

fn rewritten_item_answer(item: &Map<String, Value>, rewritten: &Value) -> HelperResult {
    rewritten_input_item(item, rewritten)
        .or_value_error()
        .map(|patched| patched.map_or(Value::Null, Value::Object))
}

fn post_shapes_answer(
    returned: Option<&[Value]>,
    pre: &[Value],
    guardrail_name: Option<&str>,
) -> Value {
    let pre_shapes = tool_call_shapes(pre);
    let shapes = post_guardrail_shapes(returned, &pre_shapes, guardrail_name);
    shapes_value(&shapes)
}

fn rewrite_answer(before: &Value, after: &Value) -> Value {
    let before_shape = single_shape(before);
    let after_shape = single_shape(after);
    let rewrite = tool_call_rewrite(&before_shape, &after_shape);
    json!({"name": rewrite.name, "arguments": rewrite.arguments})
}

fn undeliverable_answer(input: &Request) -> Value {
    let Request::UndeliverableReason {
        call_ids,
        tool_call_item_count,
        post_guardrail_tool_call_count,
        unresolved_argument_event,
        rewritten_call_ids,
        event_call_ids,
    } = input
    else {
        return Value::Null;
    };
    undeliverable_reason(&UndeliverableRewriteInput {
        call_ids,
        tool_call_item_count: *tool_call_item_count,
        post_guardrail_tool_call_count: *post_guardrail_tool_call_count,
        unresolved_argument_event: *unresolved_argument_event,
        rewritten_call_ids,
        event_call_ids,
    })
    .map_or(Value::Null, |reason| Value::String(reason.to_string()))
}

fn provenance_value(provenance: Option<&InputProvenance>) -> Value {
    provenance.map_or(Value::Null, |found| {
        json!({
            "item_for_message": found.item_for_message,
            "tainted": found.tainted,
        })
    })
}

fn shapes_value(shapes: &[ToolCallShape]) -> Value {
    let mut values = Vec::with_capacity(shapes.len());
    for shape in shapes {
        values.push(json!({"name": shape.name, "arguments": shape.arguments}));
    }
    Value::Array(values)
}

fn single_shape(call: &Value) -> ToolCallShape {
    let Some(shape) = tool_call_shapes(std::slice::from_ref(call))
        .into_iter()
        .next()
    else {
        return ToolCallShape {
            name: None,
            arguments: String::new(),
        };
    };
    shape
}
