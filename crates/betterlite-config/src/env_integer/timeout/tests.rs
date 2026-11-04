#![cfg(test)]

use super::RequestTimeout;
use rstest::rstest;
use std::collections::BTreeMap;

#[rstest]
#[case::default(None, Some((6000.0_f64, false)))]
#[case::empty(Some(""), Some((6000.0_f64, false)))]
#[case::decimal(Some("12.5"), Some((12.5_f64, true)))]
#[case::zero(Some("0"), Some((0.0_f64, true)))]
#[case::whitespace(Some(" 4 "), Some((4.0_f64, true)))]
#[case::malformed(Some("bad"), None)]
fn call_context_tests_behavior(#[case] raw: Option<&str>, #[case] expected: Option<(f64, bool)>) {
    let environment = raw
        .map(|value| BTreeMap::from([("REQUEST_TIMEOUT".to_owned(), value.to_owned())]))
        .unwrap_or_default();
    let result = RequestTimeout::resolve(&environment);
    match expected {
        Some((seconds, explicit)) => {
            let settings = result.unwrap();
            assert_eq!(settings.seconds.to_bits(), seconds.to_bits());
            assert_eq!(settings.explicitly_set, explicit);
        }
        None => assert!(result.unwrap_err().to_string().contains("REQUEST_TIMEOUT")),
    }
}
