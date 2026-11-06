use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::vigil_guard::error::VigilGuardError;
use betterlite_guardrails::vigil_guard::output::ToolCall;
use betterlite_guardrails::vigil_guard::tool_calls::set_tool_call_arguments;

use crate::support::{text, tool_call};

fn call_with(function_value: Option<GuardrailValue>) -> ToolCall {
    let mut fields = BTreeMap::new();
    fields.insert("id".to_owned(), text("call_other"));
    if let Some(function) = function_value {
        fields.insert("function".to_owned(), function);
    }
    ToolCall { fields }
}

fn function_arguments(call: &ToolCall) -> &GuardrailValue {
    let function = call.fields.get("function").expect("a function object");
    match function {
        GuardrailValue::Object(fields) => fields.get("arguments").expect("arguments"),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_) => panic!("the function field is not an object"),
    }
}

#[test]
fn replaces_arguments_and_preserves_other_fields() {
    let original = vec![tool_call("old"), tool_call("second")];
    let updated = set_tool_call_arguments(&original, 0, "new").expect("the index exists");
    assert_eq!(function_arguments(&updated[0]), &text("new"));
    assert_eq!(updated[0].fields.get("id"), Some(&text("call_1")));
    assert_eq!(updated[0].fields.get("type"), Some(&text("function")));
    assert_eq!(
        updated[0].fields.get("index"),
        Some(&GuardrailValue::SignedInteger(0))
    );
    assert_eq!(function_arguments(&updated[1]), &text("second"));
    assert_eq!(function_arguments(&original[0]), &text("old"));
}

#[test]
fn builds_a_function_object_when_missing() {
    let original = vec![call_with(None)];
    let updated = set_tool_call_arguments(&original, 0, "new").expect("the index exists");
    let function = updated[0]
        .fields
        .get("function")
        .expect("a function object");
    assert_eq!(
        function,
        &GuardrailValue::Object(BTreeMap::from([("arguments".to_owned(), text("new"),)]))
    );
    assert_eq!(updated[0].fields.get("id"), Some(&text("call_other")));
}

#[test]
fn preserves_unknown_tool_call_fields() {
    let mut original = vec![tool_call("old")];
    original[0]
        .fields
        .insert("provider_specific_fields".to_owned(), text("kept"));
    let updated = set_tool_call_arguments(&original, 0, "new").expect("the index exists");
    assert_eq!(
        updated[0].fields.get("provider_specific_fields"),
        Some(&text("kept"))
    );
}

#[test]
fn rejects_a_missing_index() {
    let original = vec![tool_call("old")];
    let error = set_tool_call_arguments(&original, 3, "new").unwrap_err();
    assert!(matches!(
        error,
        VigilGuardError::ToolCallIndex { index: 3, len: 1 }
    ));
}

#[test]
fn rejects_a_non_object_function() {
    let original = vec![call_with(Some(text("not an object")))];
    let error = set_tool_call_arguments(&original, 0, "new").unwrap_err();
    assert!(matches!(
        error,
        VigilGuardError::InvalidToolFunction { index: 0 }
    ));
}
