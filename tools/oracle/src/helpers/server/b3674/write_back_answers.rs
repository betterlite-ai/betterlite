use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_server::guardrails::anthropic_translation::scan_targets::{
    InputWriteBackTarget, ScannedText, ScannedToolCall, ToolUseInputTarget,
};
use betterlite_server::guardrails::anthropic_translation::write_back::error::ApplyError;
use betterlite_server::guardrails::anthropic_translation::write_back::{
    apply_responses_to_input, apply_tool_calls_to_input, rewritten_tool_use_input,
    write_back_message_text, write_back_system_block, write_back_tool_use,
};
use betterlite_server::guardrails::tool_call_identity::{ToolCallShape, tool_call_shapes};
use serde_json::{Value, json};

use super::requests::{ScannedCallRequest, Scenario};
use super::targets::message_target;

pub(super) fn system_block_answer(scenario: &Scenario) -> Value {
    let mut system = scenario.system.clone();
    write_back_system_block(&mut system, scenario.block_idx, &scenario.response);
    json!({"system": system})
}

pub(super) fn message_text_answer(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let mut message = scenario.message.clone();
    let target = message_target(scenario.target.as_ref())?;
    write_back_message_text(&mut message, &target, &scenario.response).or_value_error()?;
    Ok(json!({"message": message}))
}

pub(super) fn responses_answer(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let mut data = scenario.data.clone();
    let mut scanned = Vec::with_capacity(scenario.scanned_texts.len());
    for item in &scenario.scanned_texts {
        scanned.push(ScannedText {
            text: item.text.clone(),
            target: InputWriteBackTarget::from(&item.target),
        });
    }
    apply_responses_to_input(&mut data, &scenario.responses, &scanned).or_value_error()?;
    Ok(json!({"data": data}))
}

pub(super) fn tool_use_answer(scenario: &Scenario) -> Value {
    let mut message = scenario.message.clone();
    let target = ToolUseInputTarget::from(&scenario.tool_target);
    let shape = ToolCallShape {
        name: scenario.shape_name.clone(),
        arguments: scenario.arguments.clone(),
    };
    let Some(rewritten_input) = rewritten_tool_use_input(&scenario.arguments) else {
        return json!({"message": message, "input": Value::Null});
    };
    let rewritten = Value::Object(rewritten_input.clone());
    write_back_tool_use(&mut message, &target, &shape, rewritten_input);
    json!({"message": message, "input": rewritten})
}

pub(super) fn tool_calls_answer(scenario: &Scenario) -> Value {
    let mut messages = scenario.messages.clone();
    let scanned = scanned_calls(&scenario.scanned_calls);
    let pre = tool_call_shapes(&scenario.pre);
    let outcome = outcome_of(&mut messages, &scanned, &pre, scenario);
    json!({"messages": messages, "outcome": outcome})
}

fn scanned_calls(items: &[ScannedCallRequest]) -> Vec<ScannedToolCall> {
    items
        .iter()
        .map(|item| ScannedToolCall {
            tool_call: item.tool_call.clone(),
            target: ToolUseInputTarget::from(&item.tool_target),
        })
        .collect()
}

fn outcome_of(
    messages: &mut [Value],
    scanned: &[ScannedToolCall],
    pre: &[ToolCallShape],
    scenario: &Scenario,
) -> &'static str {
    let applied = apply_tool_calls_to_input(
        messages,
        scanned,
        pre,
        scenario.returned.as_deref(),
        scenario.guardrail_name.as_deref(),
    );
    match applied {
        Ok(()) => "ok",
        Err(ApplyError::Unappliable(_)) => "unappliable",
        Err(ApplyError::MessageMissing { .. }) => "message_missing",
    }
}
