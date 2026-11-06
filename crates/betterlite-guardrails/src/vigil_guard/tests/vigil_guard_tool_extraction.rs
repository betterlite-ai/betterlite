use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::vigil_guard::output::ToolCall;
use betterlite_guardrails::vigil_guard::tool_calls::tool_call_arguments;

use crate::support::{text, tool_call};

fn call_with(function_value: Option<GuardrailValue>) -> ToolCall {
    let mut fields = BTreeMap::new();
    fields.insert("id".to_owned(), text("call_other"));
    if let Some(function) = function_value {
        fields.insert("function".to_owned(), function);
    }
    ToolCall { fields }
}

fn function_with(arguments: GuardrailValue) -> GuardrailValue {
    let mut fields = BTreeMap::new();
    fields.insert("arguments".to_owned(), arguments);
    GuardrailValue::Object(fields)
}

#[test]
fn extracts_arguments_with_original_indices() {
    let calls = vec![
        tool_call("first"),
        call_with(Some(function_with(text("   ")))),
        tool_call("third"),
    ];
    assert_eq!(
        tool_call_arguments(Some(&calls)),
        vec![(0, "first".to_owned()), (2, "third".to_owned()),]
    );
}

#[test]
fn extracts_arguments_without_trimming() {
    let calls = vec![tool_call("  padded  ")];
    assert_eq!(
        tool_call_arguments(Some(&calls)),
        vec![(0, "  padded  ".to_owned())]
    );
}

#[test]
fn skips_missing_or_non_string_arguments() {
    let calls = vec![
        call_with(Some(GuardrailValue::Object(BTreeMap::new()))),
        call_with(Some(function_with(GuardrailValue::SignedInteger(3)))),
        call_with(Some(function_with(GuardrailValue::Object(BTreeMap::new())))),
    ];
    assert!(tool_call_arguments(Some(&calls)).is_empty());
}

#[test]
fn skips_tool_calls_without_a_function_object() {
    let calls = vec![call_with(None), call_with(Some(text("not an object")))];
    assert!(tool_call_arguments(Some(&calls)).is_empty());
}

#[test]
fn no_tool_calls_yield_no_pairs() {
    assert!(tool_call_arguments(None).is_empty());
    assert!(tool_call_arguments(Some(&[])).is_empty());
}
