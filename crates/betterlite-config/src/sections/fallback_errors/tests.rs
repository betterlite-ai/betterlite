#![cfg(test)]

use crate::loader::ConfigLoader;
use rstest::rstest;
use std::collections::BTreeMap;

#[rstest]
#[case::absent("{}", false)]
#[case::enabled("general_settings: {expose_fallback_errors_to_caller: true}", true)]
#[case::disabled("general_settings: {expose_fallback_errors_to_caller: false}", false)]
#[case::null("general_settings: {expose_fallback_errors_to_caller: null}", false)]
fn loads_expose_flag(#[case] source: &str, #[case] expected: bool) {
    let settings = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
    .sections
    .fallback_errors;
    assert_eq!(settings.expose_to_caller, expected);
}

#[test]
fn expose_flag_rejects_wrong_types() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings: {expose_fallback_errors_to_caller: []}")
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("expose_fallback_errors_to_caller")
    );
}
