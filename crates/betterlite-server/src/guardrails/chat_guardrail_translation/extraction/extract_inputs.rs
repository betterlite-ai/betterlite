use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{
    ScopeFlags, message_role, role_out_of_guardrail_scope,
};

use super::extracted_content::ExtractedContent;
use super::json_access::{field, image_object_url};

mod tests;

fn extract_list_items(items: &[GuardrailValue], message_index: usize, out: &mut ExtractedContent) {
    for (content_index, item) in items.iter().enumerate() {
        if let Some(GuardrailValue::Text(text)) = field(item, "text") {
            out.texts.push(text.clone());
            out.text_mappings.push((message_index, Some(content_index)));
        }
        if !matches!(field(item, "type"), Some(GuardrailValue::Text(kind)) if kind == "image_url") {
            continue;
        }
        match field(item, "image_url") {
            Some(GuardrailValue::Text(url)) => out.images.push(url.clone()),
            Some(image_url) => out.images.extend(image_object_url(image_url).cloned()),
            None => {}
        }
    }
}

pub fn extract_inputs(
    message: &GuardrailValue,
    message_index: usize,
    flags: &ScopeFlags,
    out: &mut ExtractedContent,
) {
    if role_out_of_guardrail_scope(&message_role(message), flags) {
        return;
    }
    match field(message, "content") {
        Some(GuardrailValue::Text(text)) => {
            out.texts.push(text.clone());
            out.text_mappings.push((message_index, None));
        }
        Some(GuardrailValue::Array(items)) => extract_list_items(items, message_index, out),
        _ => {}
    }
    let Some(GuardrailValue::Array(tool_calls)) = field(message, "tool_calls") else {
        return;
    };
    for (tool_call_index, tool_call) in tool_calls.iter().enumerate() {
        if let GuardrailValue::Object(fields) = tool_call {
            out.tool_calls.push(fields.clone());
            out.tool_call_mappings
                .push((message_index, tool_call_index));
        }
    }
}
