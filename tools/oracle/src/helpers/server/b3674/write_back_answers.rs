use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_server::guardrails::anthropic_translation::scan_targets::{
    InputWriteBackTarget, MessageTextTarget, ScannedText, ScannedToolCall, ToolUseInputTarget,
};
use betterlite_server::guardrails::anthropic_translation::write_back::error::ApplyError;
use betterlite_server::guardrails::anthropic_translation::write_back::{
    apply_guardrail_responses_to_input, apply_guardrail_tool_calls_to_input,
    rewritten_tool_use_input, write_back_message_text, write_back_system_block,
    write_back_tool_use,
};
use betterlite_server::guardrails::tool_call_identity::{ToolCallShape, tool_call_shapes};
use serde_json::{Value, json};

use super::requests::{
    MessageTextScenario, ResponsesScenario, SystemBlockScenario, ToolCallsScenario, ToolUseScenario,
};

pub(super) fn system_block_answer(
    scenario: &SystemBlockScenario,
) -> Result<Value, Box<HelperError>> {
    let mut system = scenario.system.clone();
    write_back_system_block(&mut system, scenario.block_idx, &scenario.response);
    Ok(json!({"system": system}))
}

pub(super) fn message_text_answer(
    scenario: &MessageTextScenario,
) -> Result<Value, Box<HelperError>> {
    let mut message = scenario.message.clone();
    let target = MessageTextTarget::from(&scenario.target);
    write_back_message_text(&mut message, &target, &scenario.response).or_value_error()?;
    Ok(json!({"message": message}))
}

pub(super) fn responses_answer(scenario: &ResponsesScenario) -> Result<Value, Box<HelperError>> {
    let mut data = scenario.data.clone();
    let mut scanned = Vec::with_capacity(scenario.scanned.len());
    for item in &scenario.scanned {
        scanned.push(ScannedText {
            text: item.text.clone(),
            target: InputWriteBackTarget::from(&item.target),
        });
    }
    apply_guardrail_responses_to_input(&mut data, &scenario.responses, &scanned)
        .or_value_error()?;
    Ok(json!({"data": data}))
}

pub(super) fn tool_use_answer(scenario: &ToolUseScenario) -> Result<Value, Box<HelperError>> {
    let mut message = scenario.message.clone();
    let target = ToolUseInputTarget {
        msg_idx: scenario.target.msg_idx,
        content_idx: scenario.target.content_idx,
    };
    let shape = ToolCallShape {
        name: scenario.shape_name.clone(),
        arguments: scenario.arguments.clone(),
    };
    let Some(rewritten_input) = rewritten_tool_use_input(&scenario.arguments) else {
        return Ok(json!({"message": message, "input": Value::Null}));
    };
    let rewritten = Value::Object(rewritten_input.clone());
    write_back_tool_use(&mut message, &target, &shape, rewritten_input);
    Ok(json!({"message": message, "input": rewritten}))
}

pub(super) fn tool_calls_answer(scenario: &ToolCallsScenario) -> Result<Value, Box<HelperError>> {
    let mut messages = scenario.messages.clone();
    let mut scanned = Vec::with_capacity(scenario.scanned.len());
    for item in &scenario.scanned {
        scanned.push(ScannedToolCall {
            tool_call: item.tool_call.clone(),
            target: ToolUseInputTarget {
                msg_idx: item.target.msg_idx,
                content_idx: item.target.content_idx,
            },
        });
    }
    let pre = tool_call_shapes(&scenario.pre);
    let outcome = match apply_guardrail_tool_calls_to_input(
        &mut messages,
        &scanned,
        &pre,
        scenario.returned.as_deref(),
        scenario.guardrail_name.as_deref(),
    ) {
        Ok(()) => "ok",
        Err(ApplyError::Unappliable(_)) => "unappliable",
        Err(ApplyError::MessageMissing { .. }) => "message_missing",
    };
    Ok(json!({"messages": messages, "outcome": outcome}))
}
