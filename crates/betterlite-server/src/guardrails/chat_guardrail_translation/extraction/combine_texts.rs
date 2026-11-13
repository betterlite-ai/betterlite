use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;
use indexmap::IndexMap;

use super::json_access::{ChoiceParts, choice_parts, field, integer_value, object_items};

mod tests;

pub type TextKey = (i64, Option<usize>);

// String content joins under (choice, None) even when empty; list content joins per item index and skips items without text.
#[must_use]
pub fn combine_streaming_texts(responses_so_far: &[Object]) -> IndexMap<TextKey, String> {
    let mut combined: IndexMap<TextKey, String> = IndexMap::new();
    for choice in responses_so_far
        .iter()
        .flat_map(|response| object_items(response, "choices"))
    {
        let Some(ChoiceParts {
            content: Some(content),
            ..
        }) = choice_parts(choice)
        else {
            continue;
        };
        let index = integer_value(field(choice, "index")).unwrap_or(0);
        match content {
            GuardrailValue::Text(text) => combined.entry((index, None)).or_default().push_str(text),
            GuardrailValue::Array(items) => combine_item_texts(items, index, &mut combined),
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Object(_) => {}
        }
    }
    combined
}

fn combine_item_texts(
    items: &[GuardrailValue],
    choice_index: i64,
    combined: &mut IndexMap<TextKey, String>,
) {
    for (item_index, item) in items.iter().enumerate() {
        let Some(GuardrailValue::Text(text)) = field(item, "text") else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        combined
            .entry((choice_index, Some(item_index)))
            .or_default()
            .push_str(text);
    }
}
