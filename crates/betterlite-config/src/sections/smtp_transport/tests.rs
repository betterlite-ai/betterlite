#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret;

use super::{Raw, Settings};
use crate::loader::ConfigError;
use crate::sections::Section as _;

fn load(variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let raw: Raw = serde_saphyr::from_str("{}").map_err(|source| ConfigError::Yaml { source })?;
    raw.convert(&environment)
}

#[test]
fn absent_settings_take_litellms_defaults() {
    let settings = load(&[]).unwrap();
    assert_eq!(settings.host, None);
    assert_eq!(settings.port, 587);
    assert_eq!(settings.username, None);
    assert!(settings.password.is_none());
    assert_eq!(settings.sender_email, None);
    assert!(!settings.use_ssl);
    assert!(settings.tls);
    assert_eq!(settings.timeout, Duration::from_secs(30));
}

#[test]
fn every_smtp_setting_is_read_from_the_environment() {
    let settings = load(&[
        ("SMTP_HOST", "smtp.example.com"),
        ("SMTP_PORT", "2525"),
        ("SMTP_USERNAME", "mailer"),
        ("SMTP_PASSWORD", "hunter2"),
        ("SMTP_SENDER_EMAIL", "proxy@example.com"),
        ("SMTP_USE_SSL", "True"),
        ("SMTP_TLS", "False"),
        ("SMTP_TIMEOUT", "12.5"),
    ])
    .unwrap();
    assert_eq!(settings.host.as_deref(), Some("smtp.example.com"));
    assert_eq!(settings.port, 2525);
    assert_eq!(settings.username.as_deref(), Some("mailer"));
    assert_eq!(
        settings.password.as_ref().map(ExposeSecret::expose_secret),
        Some("hunter2")
    );
    assert_eq!(settings.sender_email.as_deref(), Some("proxy@example.com"));
    assert!(settings.use_ssl);
    assert!(!settings.tls);
    assert_eq!(settings.timeout, Duration::from_millis(12_500));
}

#[test]
fn an_empty_variable_counts_as_unset() {
    let settings = load(&[
        ("SMTP_HOST", ""),
        ("SMTP_USERNAME", ""),
        ("SMTP_PASSWORD", ""),
        ("SMTP_SENDER_EMAIL", ""),
        ("SMTP_PORT", ""),
        ("SMTP_TIMEOUT", ""),
    ])
    .unwrap();
    assert_eq!(settings.host, None);
    assert_eq!(settings.username, None);
    assert!(settings.password.is_none());
    assert_eq!(settings.sender_email, None);
    assert_eq!(settings.port, 587);
    assert_eq!(settings.timeout, Duration::from_secs(30));
}

#[rstest]
#[case::exact_true("True", true)]
#[case::lowercase_true("true", false)]
#[case::upper_case_true("TRUE", false)]
#[case::one("1", false)]
#[case::empty("", false)]
fn use_ssl_matches_only_the_exact_true_string(#[case] value: &str, #[case] expected: bool) {
    let settings = load(&[("SMTP_USE_SSL", value)]).unwrap();
    assert_eq!(settings.use_ssl, expected);
}

#[rstest]
#[case::absent(None, true)]
#[case::exact_true(Some("True"), true)]
#[case::exact_false(Some("False"), false)]
#[case::lowercase_false(Some("false"), true)]
#[case::empty(Some(""), true)]
fn tls_stays_on_unless_the_value_is_the_exact_false_string(
    #[case] value: Option<&str>,
    #[case] expected: bool,
) {
    let settings = value.map_or_else(
        || load(&[]).unwrap(),
        |text| load(&[("SMTP_TLS", text)]).unwrap(),
    );
    assert_eq!(settings.tls, expected);
}

#[rstest]
#[case::not_a_number("SMTP_PORT", "port")]
#[case::out_of_range("SMTP_PORT", "70000")]
#[case::bad_timeout("SMTP_TIMEOUT", "soon")]
#[case::negative_timeout("SMTP_TIMEOUT", "-1")]
fn a_malformed_value_is_an_error_naming_the_variable(#[case] name: &str, #[case] value: &str) {
    assert!(matches!(
        load(&[(name, value)]),
        Err(ConfigError::Environment {
            source: betterlite_env::EnvError::Number { name: reported }
        }) if reported == name
    ));
}

#[test]
fn the_password_never_appears_in_debug_output() {
    let settings = load(&[("SMTP_PASSWORD", "hunter2")]).unwrap();
    let rendered = format!("{settings:?}");
    assert!(!rendered.contains("hunter2"));
    assert_eq!(
        settings.password.as_ref().map(ExposeSecret::expose_secret),
        Some("hunter2")
    );
}
