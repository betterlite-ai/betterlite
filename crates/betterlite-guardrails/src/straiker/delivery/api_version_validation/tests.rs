#![cfg(test)]

use rstest::rstest;

use crate::straiker::delivery::fixtures::{raw, settings, v3_raw};
use crate::straiker::delivery::settings::{ApiVersion, SettingsError, StraikerSettings};

#[rstest]
#[case::default(None, ApiVersion::V1)]
#[case::v1(Some("v1"), ApiVersion::V1)]
#[case::v3(Some("v3"), ApiVersion::V3)]
fn a_declared_version_picks_the_reader(
    #[case] configured: Option<&str>,
    #[case] expected: ApiVersion,
) {
    let mut args = raw();
    args.api_version = configured.map(str::to_owned);
    assert_eq!(settings(&args).api_version, expected);
}

#[rstest]
#[case::unknown("v2")]
#[case::empty("")]
#[case::uppercase("V3")]
fn an_unknown_version_is_rejected(#[case] value: &str) {
    let mut args = raw();
    args.api_version = Some(value.to_owned());
    let error = StraikerSettings::resolve(&args).expect_err("the version is unknown");
    assert_eq!(
        error.to_string(),
        format!("api_version must be 'v1' or 'v3'; got {value:?}")
    );
    assert!(matches!(error, SettingsError::ApiVersion { .. }));
}

#[test]
fn an_unknown_version_is_rejected_even_with_a_v3_key() {
    let mut args = v3_raw();
    args.api_version = Some("v2".to_owned());
    let error = StraikerSettings::resolve(&args).expect_err("the version is unknown");
    assert!(matches!(error, SettingsError::ApiVersion { .. }));
}

#[test]
fn the_version_is_reported_before_the_format_hint() {
    let mut args = raw();
    args.api_version = Some("v2".to_owned());
    args.format_hint = Some("anthropic".to_owned());
    let error = StraikerSettings::resolve(&args).expect_err("the version is unknown");
    assert!(matches!(error, SettingsError::ApiVersion { .. }));
}
