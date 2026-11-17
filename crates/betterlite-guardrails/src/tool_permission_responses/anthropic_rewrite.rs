use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_message_scope::Object;
use crate::tool_permission::types::{is_tool_use_block, mapping_value};

use super::evaluate::DeniedToolCall;
use super::permission_error::permission_error_result;

// LiteLLM's _modify_anthropic_content_with_permission_errors: denied blocks leave one text block of their errors
pub fn rewrite_anthropic_responses(response: &mut Object, denied: &[DeniedToolCall]) {
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
    let Some(GuardrailValue::Array(content)) = response.get("content") else {
        return;
    };
    let mut error_messages = Vec::new();
    let mut kept = Vec::with_capacity(content.len());
    for block in content {
        match denied_message(block, &errors) {
            Some(message) => error_messages.push(message),
            None => kept.push(block.clone()),
        }
    }
    // LiteLLM appends the joined errors as a text block even when no block matched, the denial list being non-empty.
    let still_has_tool_use = kept.iter().any(is_tool_use_block);
    kept.push(text_block(error_messages.join("\n")));
    response.insert("content".to_owned(), GuardrailValue::Array(kept));
    if !still_has_tool_use {
        response.insert(
            "stop_reason".to_owned(),
            GuardrailValue::Text("end_turn".to_owned()),
        );
    }
}

// A tool_use block is matched by its own id, so one without an id never finds a denial and stays.
fn denied_message(block: &GuardrailValue, errors: &BTreeMap<String, String>) -> Option<String> {
    if !is_tool_use_block(block) {
        return None;
    }
    let Some(GuardrailValue::Text(id)) = mapping_value(block, "id") else {
        return None;
    };
    errors.get(id).cloned()
}

fn text_block(text: String) -> GuardrailValue {
    let mut block = Object::new();
    block.insert("type".to_owned(), GuardrailValue::Text("text".to_owned()));
    block.insert("text".to_owned(), GuardrailValue::Text(text));
    GuardrailValue::Object(block)
}

mod tests;
