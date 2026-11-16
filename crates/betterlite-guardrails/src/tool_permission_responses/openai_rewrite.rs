use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_message_scope::Object;

use super::evaluate::DeniedToolCall;
use super::extraction::{id_text, legacy_to_tool_call};
use super::permission_error::permission_error_result;

// LiteLLM's _modify_response_with_permission_errors: each denied call leaves its error text on the message
pub fn rewrite_openai_responses(response: &mut Object, denied: &[DeniedToolCall]) {
    if denied.is_empty() {
        return;
    }
    tracing::info!(count = denied.len(), "Blocking unauthorized tool uses");
    let errors: BTreeMap<String, String> = denied
        .iter()
        .map(|denial| {
            let result = permission_error_result(&denial.call, &denial.error);
            (result.tool_use_id, result.content)
        })
        .collect();
    let Some(GuardrailValue::Array(choices)) = response.get_mut("choices") else {
        return;
    };
    for (choice_index, choice) in choices.iter_mut().enumerate() {
        rewrite_choice(choice, choice_index, &errors);
    }
}

fn rewrite_choice(
    choice: &mut GuardrailValue,
    choice_index: usize,
    errors: &BTreeMap<String, String>,
) {
    let GuardrailValue::Object(choice_fields) = choice else {
        return;
    };
    let finish_reason_is_tool = match choice_fields.get("finish_reason") {
        Some(GuardrailValue::Text(reason)) => reason == "tool_calls" || reason == "function_call",
        None | Some(_) => false,
    };
    let Some(GuardrailValue::Object(message)) = choice_fields.get_mut("message") else {
        return;
    };
    let mut error_messages = Vec::new();
    let kept = filter_tool_calls(message, errors, &mut error_messages);
    message.insert("tool_calls".to_owned(), kept);
    drop_legacy_function_call(message, choice_index, errors, &mut error_messages);
    append_error_messages(message, error_messages);
    let removed_every_tool = no_tools_left(message);
    if removed_every_tool && finish_reason_is_tool {
        choice_fields.insert(
            "finish_reason".to_owned(),
            GuardrailValue::Text("stop".to_owned()),
        );
    }
}

// LiteLLM keeps the calls no denial names and sets tool_calls to None once the last one is gone.
fn filter_tool_calls(
    message: &Object,
    errors: &BTreeMap<String, String>,
    error_messages: &mut Vec<String>,
) -> GuardrailValue {
    let Some(GuardrailValue::Array(tool_calls)) = message.get("tool_calls") else {
        return GuardrailValue::Null;
    };
    let mut kept = Vec::with_capacity(tool_calls.len());
    for tool_call in tool_calls {
        match errors.get(id_text(tool_call)) {
            Some(content) => error_messages.push(content.clone()),
            None => kept.push(tool_call.clone()),
        }
    }
    if kept.is_empty() {
        return GuardrailValue::Null;
    }
    GuardrailValue::Array(kept)
}

// The synthetic id of a legacy function call is looked up the same way extraction built it.
fn drop_legacy_function_call(
    message: &mut Object,
    choice_index: usize,
    errors: &BTreeMap<String, String>,
    error_messages: &mut Vec<String>,
) {
    let Some(function_call) = message.get("function_call") else {
        return;
    };
    let Some(legacy) = legacy_to_tool_call(function_call, choice_index) else {
        return;
    };
    if let Some(content) = errors.get(&legacy.id) {
        message.insert("function_call".to_owned(), GuardrailValue::Null);
        error_messages.push(content.clone());
    }
}

// LiteLLM joins the denials with newlines and appends them two newlines after any existing content.
fn append_error_messages(message: &mut Object, error_messages: Vec<String>) {
    if error_messages.is_empty() {
        return;
    }
    let joined = error_messages.join("\n");
    let existing = match message.get("content") {
        Some(GuardrailValue::Text(content)) if !content.is_empty() => Some(content.clone()),
        None | Some(_) => None,
    };
    let content = match existing {
        Some(existing) => format!("{existing}\n\n{joined}"),
        None => joined,
    };
    message.insert("content".to_owned(), GuardrailValue::Text(content));
}

// After the rewrite tool_calls is either None or a non-empty list, so only None means every call was removed.
fn no_tools_left(message: &Object) -> bool {
    let tool_calls_absent = matches!(message.get("tool_calls"), None | Some(GuardrailValue::Null));
    let function_call_absent = matches!(
        message.get("function_call"),
        None | Some(GuardrailValue::Null)
    );
    tool_calls_absent && function_call_absent
}

mod tests;
