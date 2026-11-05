#![cfg(test)]

use std::time::Duration;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::prompt_security_files::error::FileSanitizationError;
use crate::prompt_security_files::{FileSanitizerConfig, FileSanitizerSettings};

fn config() -> FileSanitizerConfig {
    FileSanitizerConfig {
        api_key: Some(SecretString::from("vendor-key")),
        api_base: Some("https://vendor.test".to_owned()),
        request_timeout: Duration::from_secs(5),
        file_sanitization_timeout: None,
        file_sanitization_fail_open: None,
        block_on_file_modify: None,
    }
}

#[test]
fn settings_default_to_the_litellm_values() {
    let settings = FileSanitizerSettings::try_from(config()).expect("settings");
    assert_eq!(settings.api_key.expose_secret(), "vendor-key");
    assert_eq!(settings.api_base, "https://vendor.test");
    assert_eq!(settings.request_timeout, Duration::from_secs(5));
    assert_eq!(settings.file_sanitization_timeout, Duration::from_secs(30));
    assert!(settings.fail_open);
    assert!(settings.block_on_file_modify);
    assert_eq!(settings.max_poll_attempts, 30);
    assert_eq!(settings.poll_interval, Duration::from_secs(2));
    assert_eq!(settings.max_response_bytes, 64 * 1024 * 1024);
}

#[test]
fn settings_use_explicit_values_including_false_and_zero() {
    let settings = FileSanitizerSettings::try_from(FileSanitizerConfig {
        file_sanitization_timeout: Some(Duration::ZERO),
        file_sanitization_fail_open: Some(false),
        block_on_file_modify: Some(false),
        ..config()
    })
    .expect("settings");
    assert_eq!(settings.file_sanitization_timeout, Duration::ZERO);
    assert!(!settings.fail_open);
    assert!(!settings.block_on_file_modify);
}

#[rstest]
#[case::no_key(None, Some("https://vendor.test"))]
#[case::empty_key(Some(""), Some("https://vendor.test"))]
#[case::no_base(Some("vendor-key"), None)]
#[case::empty_base(Some("vendor-key"), Some(""))]
fn settings_require_a_key_and_a_base(#[case] key: Option<&str>, #[case] base: Option<&str>) {
    let error = FileSanitizerSettings::try_from(FileSanitizerConfig {
        api_key: key.map(SecretString::from),
        api_base: base.map(str::to_owned),
        ..config()
    })
    .expect_err("missing secrets");
    assert!(matches!(error, FileSanitizationError::MissingSecrets));
}
