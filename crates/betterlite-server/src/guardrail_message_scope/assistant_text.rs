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

fn text_field<'value>(value: &'value GuardrailValue, key: &str) -> Option<&'value str> {
    match field(value, key) {
        Some(GuardrailValue::Text(text)) => Some(text),
        _ => None,
    }
}

fn has_type(value: &GuardrailValue, kind: &str) -> bool {
    match field(value, "type") {
        Some(GuardrailValue::Text(found)) => found == kind,
        _ => false,
    }
}

fn array_field<'value>(
    value: &'value GuardrailValue,
    key: &str,
) -> Option<&'value Vec<GuardrailValue>> {
    match field(value, key) {
        Some(GuardrailValue::Array(items)) => Some(items),
        _ => None,
    }
}

fn chat_text(response: &GuardrailValue) -> Option<String> {
    let message = field(array_field(response, "choices")?.first()?, "message")?;
    let text = content_to_text(field(message, "content")?);
    (!text.is_empty()).then_some(text)
}

// The texts of the blocks of one type, joined; none when they join to nothing.
fn joined_texts<'value, Blocks>(blocks: Blocks, kind: &str) -> Option<String>
where
    Blocks: Iterator<Item = &'value GuardrailValue>,
{
    let text: String = blocks
        .filter(|block| has_type(block, kind))
        .filter_map(|block| text_field(block, "text"))
        .collect();
    (!text.is_empty()).then_some(text)
}

fn responses_text(response: &GuardrailValue) -> Option<String> {
    let chunks = array_field(response, "output")?
        .iter()
        .filter(|item| has_type(item, "message"))
        .filter_map(|item| array_field(item, "content"))
        .flatten();
    joined_texts(chunks, "output_text")
}

// The first shape that yields text wins: chat choices, then Anthropic content blocks, then Responses output.
#[must_use]
pub fn assistant_text_from_response(response: &GuardrailValue) -> Option<String> {
    chat_text(response)
        .or_else(|| {
            let blocks = array_field(response, "content")?;
            joined_texts(blocks.iter(), "text")
        })
        .or_else(|| responses_text(response))
}
