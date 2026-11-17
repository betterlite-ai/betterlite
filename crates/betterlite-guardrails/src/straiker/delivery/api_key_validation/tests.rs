#![cfg(test)]

use secrecy::SecretString;

use crate::straiker::delivery::fixtures::{raw, settings, v3_raw};
use crate::straiker::delivery::settings::{ApiVersion, SettingsError, StraikerSettings};

#[test]
fn an_empty_api_key_is_rejected() {
    let mut args = raw();
    args.api_key = SecretString::from("");
    let error = StraikerSettings::resolve(&args).expect_err("the key is empty");
    assert!(matches!(error, SettingsError::EmptyApiKey));
    assert_eq!(error.to_string(), "api_key must be non-empty");
}

#[test]
fn a_blank_api_key_is_a_key() {
    let mut args = raw();
    args.api_key = SecretString::from(" ");
    let resolved = settings(&args);
    assert_eq!(resolved.api_version, ApiVersion::V1);
}

#[test]
fn an_empty_key_is_reported_before_the_fallback_value() {
    let mut args = raw();
    args.api_key = SecretString::from("");
    args.unreachable_fallback = "sometimes".to_owned();
    let error = StraikerSettings::resolve(&args).expect_err("the key is empty");
    assert!(matches!(error, SettingsError::EmptyApiKey));
}

#[test]
fn an_empty_key_is_reported_before_the_api_version() {
    let mut args = v3_raw();
    args.api_key = SecretString::from("");
    args.api_version = Some("v9".to_owned());
    let error = StraikerSettings::resolve(&args).expect_err("the key is empty");
    assert!(matches!(error, SettingsError::EmptyApiKey));
}
