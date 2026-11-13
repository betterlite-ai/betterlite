use std::collections::BTreeSet;

use betterlite_server::guardrails::anthropic_translation::scan_targets::is_client_tool_use;
use betterlite_server::guardrails::responses_translation::input_provenance::{
    InputProvenance, input_item_provenance, item_rewrite_field, rewritten_input_item,
};
use betterlite_server::guardrails::tool_call_identity::{
    ToolCallShape, post_guardrail_tool_call_shapes, tool_call_rewrite, tool_call_shapes,
    undeliverable_reason,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
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
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(match input {
            Input::InputItemProvenance {
                raw_input,
                expected_messages,
            } => {
                let provenance = input_item_provenance(raw_input, expected_messages)
                    .or_value_error()?;
                provenance_value(provenance.as_ref())
            }
            Input::ItemRewriteField { item } => item_rewrite_field(item)
                .map_or(Value::Null, |field| Value::String(field.to_owned())),
            Input::RewrittenItem { item, rewritten } => {
                let patched = rewritten_input_item(item, rewritten).or_value_error()?;
                patched.map_or(Value::Null, Value::Object)
            }
            Input::ToolCallShapes { calls } => shapes_value(&tool_call_shapes(calls)),
            Input::PostGuardrailShapes {
                returned,
                pre,
                guardrail_name,
            } => {
                let pre_shapes = tool_call_shapes(pre);
                let shapes = post_guardrail_tool_call_shapes(
                    returned.as_deref(),
                    &pre_shapes,
                    guardrail_name.as_deref(),
                );
                shapes_value(&shapes)
            }
            Input::ToolCallRewrite { before, after } => {
                let before_shape = single_shape(before);
                let after_shape = single_shape(after);
                shape_value(&tool_call_rewrite(&before_shape, &after_shape))
            }
            Input::UndeliverableReason {
                call_ids,
                tool_call_item_count,
                post_guardrail_tool_call_count,
                unresolved_argument_event,
                rewritten_call_ids,
                event_call_ids,
            } => undeliverable_reason(
                call_ids,
                *tool_call_item_count,
                *post_guardrail_tool_call_count,
                *unresolved_argument_event,
                rewritten_call_ids,
                event_call_ids,
            )
            .map_or(Value::Null, |reason| Value::String(reason.to_string())),
            Input::ClientToolUse { block } => Value::Bool(is_client_tool_use(block)),
        })
    }
}

fn provenance_value(provenance: Option<&InputProvenance>) -> Value {
    provenance.map_or(Value::Null, |provenance| {
        json!({
            "item_for_message": provenance.item_for_message,
            "tainted": provenance.tainted,
        })
    })
}

fn shapes_value(shapes: &[ToolCallShape]) -> Value {
    Value::Array(shapes.iter().map(shape_value).collect())
}

fn shape_value(shape: &ToolCallShape) -> Value {
    json!({"name": shape.name, "arguments": shape.arguments})
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
