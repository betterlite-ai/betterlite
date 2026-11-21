#![cfg(test)]

use super::masked::{masked_texts, response_has_masked_output};
use super::support::strings;
use crate::bedrock_records::Response;

fn response(json: &str) -> Response {
    serde_json::from_str(json).expect("the guardrail response parses")
}

#[test]
fn output_text_marks_the_response_as_masked() {
    let parsed = response(r#"{"action": "NONE", "output": [{"text": "MASKED"}]}"#);
    assert!(response_has_masked_output(&parsed));
}

#[test]
fn outputs_text_marks_the_response_as_masked() {
    let parsed = response(r#"{"action": "NONE", "outputs": [{"text": "MASKED"}]}"#);
    assert!(response_has_masked_output(&parsed));
}

#[test]
fn empty_texts_are_not_masking() {
    let parsed = response(r#"{"output": [{"text": ""}], "outputs": [{"text": ""}]}"#);
    assert!(!response_has_masked_output(&parsed));
}

#[test]
fn a_passing_action_without_content_is_not_masking() {
    let parsed = response(r#"{"action": "NONE", "usage": {"textUnits": 3}}"#);
    assert!(!response_has_masked_output(&parsed));
}

#[test]
fn an_item_without_text_is_not_masking() {
    let parsed = response(r#"{"output": [{}], "outputs": [{"text": null}]}"#);
    assert!(!response_has_masked_output(&parsed));
}

#[test]
fn masked_texts_prefers_the_output_field() {
    let parsed = response(r#"{"output": [{"text": "MASKED"}], "outputs": [{"text": "OTHER"}]}"#);
    assert_eq!(masked_texts(&parsed), strings(&["MASKED"]));
}

#[test]
fn masked_texts_falls_back_to_outputs() {
    let parsed = response(r#"{"outputs": [{"text": "MASKED"}]}"#);
    assert_eq!(masked_texts(&parsed), strings(&["MASKED"]));
}

#[test]
fn an_empty_output_list_still_uses_outputs() {
    let parsed = response(r#"{"output": [], "outputs": [{"text": "MASKED"}]}"#);
    assert_eq!(masked_texts(&parsed), strings(&["MASKED"]));
}

#[test]
fn masked_texts_skips_empty_entries() {
    let parsed = response(r#"{"output": [{"text": ""}, {"text": "kept"}, {}]}"#);
    assert_eq!(masked_texts(&parsed), strings(&["kept"]));
}
