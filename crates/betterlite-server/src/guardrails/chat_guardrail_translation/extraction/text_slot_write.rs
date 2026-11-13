use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::apply_error::ApplyError;
use super::json_access::field_mut;

// `holder` is the message (or choice message) that owns the content; null or absent content is left alone.
pub fn write_text_slot(
    holder: &mut GuardrailValue,
    entry_index: usize,
    content_index: Option<usize>,
    response: &str,
) -> Result<(), ApplyError> {
    let Some(content) = field_mut(holder, "content") else {
        return Ok(());
    };
    match (content, content_index) {
        (GuardrailValue::Text(text), None) => response.clone_into(text),
        (GuardrailValue::Array(items), Some(slot)) => {
            let Some(GuardrailValue::Object(item)) = items.get_mut(slot) else {
                return Err(ApplyError::MissingContentItem {
                    index: entry_index,
                    content_index: slot,
                });
            };
            item.insert("text".to_owned(), GuardrailValue::Text(response.to_owned()));
        }
        _ => {}
    }
    Ok(())
}
