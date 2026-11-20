use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{ScanScope, ScopeFlags};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_server::guardrails::anthropic_translation::input_scan::images::image_sources;
use betterlite_server::guardrails::anthropic_translation::input_scan::{
    extract_input_text_and_images, extract_top_level_system_text, get_structured_messages,
};
use betterlite_server::guardrails::anthropic_translation::output_scan::apply_responses_to_output;
use betterlite_server::guardrails::anthropic_translation::post_call_hook::post_call_hook_response;
use betterlite_server::guardrails::anthropic_translation::scan_targets::{
    InputWriteBackTarget, MessageTextTarget, ScannedText,
};
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::requests::{Flags, Scenario};

pub(super) fn extract_answer(scenario: &Scenario) -> Value {
    let found = extract_input_text_and_images(
        &scenario.message,
        scenario.msg_idx,
        scope_flags(&scenario.flags),
    );
    json!({
        "texts": found.scanned.iter().map(text_view).collect::<Vec<Value>>(),
        "images": found.images,
        "tool_calls": found
            .tool_calls
            .iter()
            .map(|call| call.tool_call.clone())
            .collect::<Vec<Value>>(),
    })
}

pub(super) fn image_sources_answer(scenario: &Scenario) -> Value {
    let Some(block) = scenario.block.as_object() else {
        return json!({"images": []});
    };
    json!({"images": image_sources(block)})
}

pub(super) fn top_level_system_answer(scenario: &Scenario) -> Value {
    let found = extract_top_level_system_text(&scenario.hoisted);
    json!({
        "texts": found.iter().map(text_view).collect::<Vec<Value>>()
    })
}

pub(super) fn structured_messages_answer(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let runtime = current_runtime()?;
    let messages = runtime
        .block_on(get_structured_messages(&scenario.data))
        .or_value_error()?;
    Ok(json!({"messages": messages.map_or(Value::Null, Value::Array)}))
}

pub(super) fn output_apply_answer(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let mut content = scenario.content.clone();
    apply_responses_to_output(&mut content, &scenario.responses, &scenario.mappings)
        .or_value_error()?;
    Ok(json!({"content": content}))
}

pub(super) fn post_call_hook_answer(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let response = scenario.response.as_ref().unwrap_or(&GuardrailValue::Null);
    let translated = post_call_hook_response(response).or_value_error()?;
    Ok(json!({
        "response": translated
    }))
}

const fn scope_flags(flags: &Flags) -> ScopeFlags {
    ScopeFlags {
        skip_system_message: flags.skip_system_message.0,
        skip_tool_message: flags.skip_tool_message.0,
        scan: if flags.scan_only_tool_results.0 {
            ScanScope::ToolResultsOnly
        } else {
            ScanScope::AllMessages
        },
    }
}

fn text_view(item: &ScannedText) -> Value {
    json!({
        "text": item.text,
        "target": target_view(&item.target)
    })
}

fn target_view(target: &InputWriteBackTarget) -> Value {
    match target {
        InputWriteBackTarget::SystemString => target_json("system_string", 0, 0, 0),
        InputWriteBackTarget::SystemBlockText { block_idx } => {
            target_json("system_block_text", 0, 0, *block_idx)
        }
        InputWriteBackTarget::Message(MessageTextTarget::MessageContent { msg_idx }) => {
            target_json("message_content", *msg_idx, 0, 0)
        }
        InputWriteBackTarget::Message(MessageTextTarget::ContentBlockText {
            msg_idx,
            content_idx,
        }) => target_json("content_block_text", *msg_idx, *content_idx, 0),
        InputWriteBackTarget::Message(MessageTextTarget::ToolResultString {
            msg_idx,
            content_idx,
        }) => target_json("tool_result_string", *msg_idx, *content_idx, 0),
        InputWriteBackTarget::Message(MessageTextTarget::ToolResultBlockText {
            msg_idx,
            content_idx,
            block_idx,
        }) => target_json("tool_result_block_text", *msg_idx, *content_idx, *block_idx),
    }
}

fn target_json(kind: &str, msg_idx: usize, content_idx: usize, block_idx: usize) -> Value {
    json!({
        "kind": kind,
        "msg_idx": msg_idx,
        "content_idx": content_idx,
        "block_idx": block_idx
    })
}

fn current_runtime() -> Result<Runtime, Box<HelperError>> {
    let mut builder = Builder::new_current_thread();
    builder.enable_all();
    builder.build().or_config_error()
}
