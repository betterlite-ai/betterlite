use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::content_text::content_to_text;

mod tests;

// A null value reads as absent, the way LiteLLM's attribute-or-key lookup treats None.
fn field<'value>(value: &'value GuardrailValue, key: &str) -> Option<&'value GuardrailValue> {
    let GuardrailValue::Object(fields) = value else {
        return None;
    };
    fields
        .get(key)
        .filter(|found| !matches!(found, GuardrailValue::Null))
}

fn non_empty_text<'value>(value: &'value GuardrailValue, key: &str) -> Option<&'value str> {
    match field(value, key) {
        Some(GuardrailValue::Text(text)) if !text.is_empty() => Some(text),
        _ => None,
    }
}

fn has_type(value: &GuardrailValue, kind: &str) -> bool {
    match field(value, "type") {
        Some(GuardrailValue::Text(found)) => found == kind,
        _ => false,
    }
}

fn chat_text(response: &GuardrailValue) -> Option<String> {
    let Some(GuardrailValue::Array(choices)) = field(response, "choices") else {
        return None;
    };
    let message = field(choices.first()?, "message")?;
    let text = content_to_text(field(message, "content")?);
    (!text.is_empty()).then_some(text)
}

fn anthropic_text(response: &GuardrailValue) -> Option<String> {
    let Some(GuardrailValue::Array(blocks)) = field(response, "content") else {
        return None;
    };
    let text: String = blocks
        .iter()
        .filter(|block| has_type(block, "text"))
        .filter_map(|block| non_empty_text(block, "text"))
        .collect();
    (!text.is_empty()).then_some(text)
}

fn responses_text(response: &GuardrailValue) -> Option<String> {
    let Some(GuardrailValue::Array(output)) = field(response, "output") else {
        return None;
    };
    let text: String = output
        .iter()
        .filter(|item| has_type(item, "message"))
        .filter_map(|item| match field(item, "content") {
            Some(GuardrailValue::Array(chunks)) => Some(chunks),
            _ => None,
        })
        .flatten()
        .filter(|chunk| has_type(chunk, "output_text"))
        .filter_map(|chunk| non_empty_text(chunk, "text"))
        .collect();
    (!text.is_empty()).then_some(text)
}

// The first shape that yields text wins: chat choices, then Anthropic content blocks, then Responses output.
#[must_use]
pub fn assistant_text_from_response(response: &GuardrailValue) -> Option<String> {
    chat_text(response)
        .or_else(|| anthropic_text(response))
        .or_else(|| responses_text(response))
}
