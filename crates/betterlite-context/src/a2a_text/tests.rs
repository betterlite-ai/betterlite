#![cfg(test)]
use super::{A2aValue, extract_text_from_a2a_message, extract_text_from_a2a_response};
use rstest::rstest;
fn parsed(source: &str) -> A2aValue {
    serde_json::from_str(source).unwrap()
}
#[rstest]
#[case::a2a_text_parts("parts")]
#[case::a2a_depth_limit("depth")]
#[case::a2a_result_priority("priority")]
#[case::a2a_wrong_shapes("shapes")]
fn a2a_text_cases(#[case] case: &str) {
    if case == "parts" || case == "depth" {
        let message = parsed(
            r#"{"parts":[{"kind":"text","text":""},{"kind":"text","text":"one","parts":[{"kind":"text","text":"ignored"}]},{"kind":"nested","parts":[{"kind":"text","text":"two"}]},{"kind":"nested","parts":[]}]}"#,
        );
        assert_eq!(
            extract_text_from_a2a_message(Some(&message), 0, 10).unwrap(),
            " one two"
        );
        assert_eq!(extract_text_from_a2a_message(None, 0, 10).unwrap(), "");
        assert_eq!(
            extract_text_from_a2a_message(Some(&message), 10, 10).unwrap(),
            ""
        );
    } else if case == "priority" {
        let array_fallback = parsed(
            r#"{"result":{"message":[],"artifact":{"parts":[{"kind":"text","text":"array"}]}}}"#,
        );
        assert_eq!(
            extract_text_from_a2a_response(array_fallback.as_object().unwrap(), 10).unwrap(),
            "array"
        );
        let object_fallback = parsed(
            r#"{"result":{"message":{},"artifact":{"parts":[{"kind":"text","text":"object"}]}}}"#,
        );
        assert_eq!(
            extract_text_from_a2a_response(object_fallback.as_object().unwrap(), 10).unwrap(),
            "object"
        );
        let string_fallback = parsed(
            r#"{"result":{"message":"","artifact":{"parts":[{"kind":"text","text":"string"}]}}}"#,
        );
        assert_eq!(
            extract_text_from_a2a_response(string_fallback.as_object().unwrap(), 10).unwrap(),
            "string"
        );
    } else {
        assert_eq!(case, "shapes");
    }
}
