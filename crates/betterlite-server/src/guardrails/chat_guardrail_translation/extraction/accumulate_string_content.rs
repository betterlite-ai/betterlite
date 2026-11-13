use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;
use indexmap::IndexMap;

use super::json_access::{ChoiceParts, choice_parts, field, integer_value, object_items};

mod tests;

// Keyed by the choice's own index, in the order each index first appears; list content and empty strings add nothing.
#[must_use]
pub fn accumulate_by_choice_index(responses_so_far: &[Object]) -> IndexMap<i64, String> {
    let mut accumulated: IndexMap<i64, String> = IndexMap::new();
    for choice in responses_so_far
        .iter()
        .flat_map(|response| object_items(response, "choices"))
    {
        let Some(ChoiceParts {
            content: Some(GuardrailValue::Text(text)),
            ..
        }) = choice_parts(choice)
        else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let index = integer_value(field(choice, "index")).unwrap_or(0);
        accumulated.entry(index).or_default().push_str(text);
    }
    accumulated
}
