#![cfg(test)]

use rstest::rstest;

use crate::models::decode::{DecodeError, decode};
use crate::models::settings::LensSettings;

#[rstest]
#[case::duplicate_ids(
    r#""context": "Cite sources.", "checks": [{"id": "tone", "instruction": "Calm."}, {"id": "tone", "instruction": "Kind."}]"#,
    "Each check must have a unique ID"
)]
#[case::blank_context_without_enabled_check(
    r#""context": "  ", "checks": [{"id": "tone", "instruction": "Calm.", "enabled": false}]"#,
    "Describe expected behavior or add an enabled check"
)]
#[case::reserved_id(
    r#""checks": [{"id": "expected_behavior", "instruction": "Calm."}]"#,
    "expected_behavior is reserved for the behavior description"
)]
fn each_check_rule_has_its_own_message(#[case] fields: &str, #[case] message: &str) {
    let json = format!(r#"{{"name": "lens", "model": "gpt-4o", {fields}}}"#);
    let DecodeError::Invalid(rule) = decode::<LensSettings>(json.as_bytes()).unwrap_err() else {
        panic!("the settings must fail a check rule");
    };
    assert_eq!(rule.to_string(), message);
}

#[test]
fn analysis_runs_the_behavior_check_then_enabled_checks() {
    let json = r#"{"name": "lens", "model": "gpt-4o", "context": "Cite sources.", "checks": [
        {"id": "tone", "instruction": "Calm."}, {"id": "length", "instruction": "Short.", "enabled": false}]}"#;
    let settings = decode::<LensSettings>(json.as_bytes()).unwrap();
    let ids: Vec<String> = settings
        .analysis_checks()
        .into_iter()
        .map(|check| check.id)
        .collect();
    assert_eq!(ids, ["expected_behavior", "tone"]);
}
