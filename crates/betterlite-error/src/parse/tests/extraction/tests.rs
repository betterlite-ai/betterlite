#![cfg(test)]

use super::*;

#[rstest]
#[case::extraction()]
fn error_facts_behavior_tests_behavior() {
    let facts = extract_facts(br#"{"message":"provider failed"}"#, Some(502));
    assert_eq!(facts.status_code, Some(502));
    assert!(!facts.status_code_is_synthesized);
    assert_eq!(facts.message.as_deref(), Some("provider failed"));
    assert_eq!(
        facts.body.map(|body| body.message),
        Some(Field::Value(String::from("provider failed")))
    );
    let synthesized = extract_facts(br#"{"message":"provider failed"}"#, None);
    assert_eq!(synthesized.status_code, None);
    assert!(synthesized.status_code_is_synthesized);
}
