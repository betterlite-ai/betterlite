use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;

use super::json_access::{field, integer_value, object_items};

mod tests;

// A choice without an integer index counts as choice 0, as LiteLLM's typed choices default.
#[must_use]
pub fn narrowed_to_choice(response: &Object, index: i64) -> Object {
    let narrowed = object_items(response, "choices")
        .iter()
        .filter(|choice| integer_value(field(choice, "index")).unwrap_or(0) == index)
        .cloned()
        .collect();
    let mut copy = response.clone();
    copy.insert("choices".to_owned(), GuardrailValue::Array(narrowed));
    copy
}
