use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::support::{array, object, text, unsigned};
use crate::guardrail_hooks::errors::detail_message;

#[test]
fn a_nested_error_message_names_the_detail() {
    let error = object(vec![("message", text("nested"))]);
    assert_eq!(detail_message(&object(vec![("error", error)])), "nested");
}

#[test]
fn a_detail_without_a_message_keeps_its_json_text() {
    assert_eq!(detail_message(&GuardrailValue::Boolean(true)), "true");
    assert_eq!(detail_message(&array(vec![unsigned(1)])), "[1]");
    assert_eq!(
        detail_message(&object(vec![("code", unsigned(7))])),
        "{\"code\":7}"
    );
}
