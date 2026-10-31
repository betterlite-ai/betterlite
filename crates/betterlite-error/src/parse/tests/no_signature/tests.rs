#![cfg(test)]

use super::*;

#[rstest]
#[case::model_not_mapped("ModelNotMappedError")]
#[case::zone_info_not_found("ZoneInfoNotFoundError")]
fn error_facts_behavior_tests_behavior(#[case] name: &str) {
    let text = format!("litellm.{name}");
    assert!(matches!(
        extract_exception(None, &text, "model", "provider", None),
        Err(ParseError::Signature { name: error_name }) if error_name == name
    ));
}
