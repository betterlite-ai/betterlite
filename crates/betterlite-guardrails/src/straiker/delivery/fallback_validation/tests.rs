#![cfg(test)]

use rstest::rstest;

use crate::generic_guardrail_contracts::UnreachableFallback;
use crate::straiker::delivery::fixtures::{raw, settings};
use crate::straiker::delivery::settings::{SettingsError, StraikerSettings};

#[rstest]
#[case::fail_closed("fail_closed", UnreachableFallback::FailClosed)]
#[case::fail_open("fail_open", UnreachableFallback::FailOpen)]
fn a_known_fallback_value_resolves(#[case] value: &str, #[case] expected: UnreachableFallback) {
    let mut args = raw();
    args.unreachable_fallback = value.to_owned();
    assert_eq!(settings(&args).unreachable_fallback, expected);
}

#[rstest]
#[case::mixed_case("Fail_Open")]
#[case::empty("")]
#[case::short("open")]
#[case::hyphenated("fail-closed")]
fn an_unknown_fallback_value_is_rejected(#[case] value: &str) {
    let mut args = raw();
    args.unreachable_fallback = value.to_owned();
    let error = StraikerSettings::resolve(&args).expect_err("the fallback is unknown");
    assert_eq!(
        error.to_string(),
        format!("unreachable_fallback must be 'fail_open' or 'fail_closed'; got {value:?}")
    );
    assert!(matches!(
        error,
        SettingsError::UnreachableFallback { value: rejected } if rejected == value
    ));
}

#[test]
fn the_fallback_is_reported_before_the_api_version() {
    let mut args = raw();
    args.unreachable_fallback = "maybe".to_owned();
    args.api_version = Some("v9".to_owned());
    let error = StraikerSettings::resolve(&args).expect_err("the fallback is unknown");
    assert!(matches!(error, SettingsError::UnreachableFallback { .. }));
}
