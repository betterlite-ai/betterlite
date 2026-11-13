use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::convert_tool_call::convert_tool_call_to_dict;
use super::extracted_content::ExtractedContent;
use super::json_access::{choice_parts, field, image_object_url};

mod tests;

fn extract_list_content(items: &[GuardrailValue], choice_index: usize, out: &mut ExtractedContent) {
    for (content_index, item) in items.iter().enumerate() {
        if let Some(GuardrailValue::Text(text)) = field(item, "text")
            && !text.is_empty()
        {
            out.texts.push(text.clone());
            out.text_mappings.push((choice_index, Some(content_index)));
        }
        let is_image =
            matches!(field(item, "type"), Some(GuardrailValue::Text(kind)) if kind == "image_url");
        if is_image && let Some(url) = field(item, "image_url").and_then(image_object_url) {
            out.images.push(url.clone());
        }
    }
}

// Empty content adds nothing, and an image URL is only read from an object, never from a bare string.
pub fn extract_output(choice: &GuardrailValue, choice_index: usize, out: &mut ExtractedContent) {
    let Some(parts) = choice_parts(choice) else {
        return;
    };
    match parts.content {
        Some(GuardrailValue::Text(text)) if !text.is_empty() => {
            out.texts.push(text.clone());
            out.text_mappings.push((choice_index, None));
        }
        Some(GuardrailValue::Array(items)) => extract_list_content(items, choice_index, out),
        _ => {}
    }
    for (tool_call_index, tool_call) in parts.tool_calls.iter().enumerate() {
        if let Some(dict) = convert_tool_call_to_dict(tool_call) {
            out.tool_calls.push(dict);
            out.tool_call_mappings.push((choice_index, tool_call_index));
        }
    }
}
