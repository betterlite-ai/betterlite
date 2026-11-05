#![cfg(test)]

use crate::loader::ConfigLoader;
use rstest::rstest;
use std::collections::BTreeMap;

#[rstest]
#[case::absent("{}", false)]
#[case::enabled("general_settings: {include_call_id_in_error_body: true}", true)]
#[case::disabled("general_settings: {include_call_id_in_error_body: false}", false)]
#[case::null("general_settings: {include_call_id_in_error_body: null}", false)]
fn error_normalization_tests_behavior(#[case] source: &str, #[case] expected: bool) {
    let settings = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
    .sections
    .error_body_call_id;
    assert_eq!(settings.include_call_id_in_error_body, expected);
}

#[test]
fn call_id_setting_rejects_wrong_types() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings: {include_call_id_in_error_body: []}")
    .unwrap_err();
    assert!(error.to_string().contains("include_call_id_in_error_body"));
}
