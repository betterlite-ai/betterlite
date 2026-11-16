use std::collections::BTreeSet;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::guardrails::streaming_rewrite::failure::UndeliverableStreamRewrite;
use betterlite_guardrails::guardrail_message_scope::Object;

use super::response::ChatResponse;
use super::tool_call_fragments::{FragmentPosition, function_tool_call_fragments};
use super::tool_call_shapes::{ToolCallShape, function_tool_call_shapes};
use crate::guardrails::chat_guardrail_translation::extraction::json_access::{
    field, field_mut, integer_value, object_items,
};

mod tests;

fn function_mut(responses: &mut [Object], at: FragmentPosition) -> Option<&mut Object> {
    let GuardrailValue::Array(choices) = responses.get_mut(at.chunk)?.get_mut("choices")? else {
        return None;
    };
    let delta = field_mut(choices.get_mut(at.choice)?, "delta")?;
    let GuardrailValue::Array(calls) = field_mut(delta, "tool_calls")? else {
        return None;
    };
    let GuardrailValue::Object(function) = field_mut(calls.get_mut(at.tool_call)?, "function")?
    else {
        return None;
    };
    Some(function)
}

// The rewritten name and full arguments go to the call's first fragment and its later fragments lose their arguments.
fn write_fragments(
    responses: &mut [Object],
    fragments: &[FragmentPosition],
    (name, arguments): &ToolCallShape,
) {
    for (position, &at) in fragments.iter().enumerate() {
        let Some(function) = function_mut(responses, at) else {
            continue;
        };
        if position == 0 {
            let name_value = name
                .clone()
                .map_or(GuardrailValue::Null, GuardrailValue::Text);
            function.insert("name".to_owned(), name_value);
            function.insert(
                "arguments".to_owned(),
                GuardrailValue::Text(arguments.clone()),
            );
        } else {
            function.insert("arguments".to_owned(), GuardrailValue::Text(String::new()));
        }
    }
}

// A rewrite on a multi-choice stream, or one whose fragments do not line up with the rebuilt calls, is refused before anything is written.
pub fn write_ended_tool_rewrites(
    responses_so_far: &mut [Object],
    guardrailed: &ChatResponse,
    pre_guardrail_tool_calls: &[ToolCallShape],
    guardrail_name: &str,
) -> Result<(), UndeliverableStreamRewrite> {
    let post_guardrail_tool_calls = function_tool_call_shapes(guardrailed);
    if post_guardrail_tool_calls == pre_guardrail_tool_calls {
        return Ok(());
    }
    let choice_indices: BTreeSet<i64> = responses_so_far
        .iter()
        .flat_map(|response| object_items(response, "choices"))
        .map(|choice| integer_value(field(choice, "index")).unwrap_or(0))
        .collect();
    let fragments = function_tool_call_fragments(responses_so_far);
    let refusal = |reason: String| UndeliverableStreamRewrite {
        guardrail_name: guardrail_name.to_owned(),
        reason,
    };
    if choice_indices.len() != 1 {
        return Err(refusal(format!(
            "the stream carries {} choices and tool-call rewrites are only written back on single-choice streams",
            choice_indices.len()
        )));
    }
    if fragments.len() != post_guardrail_tool_calls.len() {
        return Err(refusal(format!(
            "the guardrail returned {} tool calls for a stream that carried {}",
            post_guardrail_tool_calls.len(),
            fragments.len()
        )));
    }
    for ((before, after), group) in pre_guardrail_tool_calls
        .iter()
        .zip(&post_guardrail_tool_calls)
        .zip(&fragments)
    {
        if before != after {
            write_fragments(responses_so_far, group, after);
        }
    }
    Ok(())
}
