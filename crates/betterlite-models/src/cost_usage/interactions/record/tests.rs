#![cfg(test)]

use super::{InteractionsUsage, is_interactions_usage_object};

fn parse(source: &str) -> InteractionsUsage {
    serde_json::from_str(source).expect("valid JSON object")
}

#[test]
fn total_input_tokens_present_is_recognized() {
    let usage = parse(r#"{"total_input_tokens":5}"#);
    assert!(is_interactions_usage_object(&usage));
}

#[test]
fn total_output_tokens_present_is_recognized() {
    let usage = parse(r#"{"total_output_tokens":5}"#);
    assert!(is_interactions_usage_object(&usage));
}

#[test]
fn prompt_tokens_key_disqualifies_even_when_absent_qualifiers_would_match() {
    let usage = parse(r#"{"prompt_tokens":10,"total_input_tokens":5}"#);
    assert!(!is_interactions_usage_object(&usage));
}

#[test]
fn null_prompt_tokens_still_disqualifies() {
    let usage = parse(r#"{"prompt_tokens":null,"total_input_tokens":5}"#);
    assert!(!is_interactions_usage_object(&usage));
}

#[test]
fn prompt_tokens_with_truthy_non_array_modality_is_not_recognized() {
    let usage =
        parse(r#"{"prompt_tokens":1,"total_input_tokens":1,"input_tokens_by_modality":"x"}"#);
    assert!(!is_interactions_usage_object(&usage));
}

#[test]
fn input_tokens_key_disqualifies() {
    let usage = parse(r#"{"input_tokens":1,"total_output_tokens":5}"#);
    assert!(!is_interactions_usage_object(&usage));
}

#[test]
fn neither_qualifier_present_is_not_recognized() {
    let usage = parse(r#"{"total_tool_use_tokens":5}"#);
    assert!(!is_interactions_usage_object(&usage));
}
