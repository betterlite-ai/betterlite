#![cfg(test)]

use super::{
    SecretManagerReadState, manager_read_is_consulted, manager_read_is_enabled,
    normalize_nonempty_secret_str, read_boolean_text,
};

use crate::secret_types::SecretAccessMode;
use betterlite_env::EnvError;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

#[rstest]
#[case::true_text(Some(" TRUE "), Some(true))]
#[case::false_text(Some("False"), Some(false))]
#[case::yes_text(Some("yes"), Some(true))]
#[case::absent(None, None)]
#[case::empty(Some(""), None)]
fn boolean_text_uses_the_shared_environment_policy(
    #[case] value: Option<&str>,
    #[case] expected: Option<bool>,
) {
    assert_eq!(read_boolean_text("SETTING", value).unwrap(), expected);
}

#[test]
fn invalid_boolean_text_names_the_variable_without_the_value() {
    let error = read_boolean_text("SETTING", Some("private-value")).unwrap_err();
    assert_eq!(
        error,
        EnvError::Bool {
            name: "SETTING".to_owned()
        }
    );
    assert!(!error.to_string().contains("private-value"));
}

#[rstest]
#[case::value(Some("  secret \n"), Some("secret"))]
#[case::empty(Some(""), None)]
#[case::whitespace(Some(" \t\n"), None)]
#[case::absent(None, None)]
#[case::unicode(Some("\u{1c}\u{a0}secret\u{1f}"), Some("secret"))]
fn normalizes_empty_secret_values(#[case] input: Option<&str>, #[case] expected: Option<&str>) {
    let secret = input.map(SecretString::from);
    let normalized = normalize_nonempty_secret_str(secret.as_ref());
    assert_eq!(
        normalized.as_ref().map(ExposeSecret::expose_secret),
        expected
    );
    assert!(!format!("{normalized:?}").contains("secret"));
}

#[rstest]
#[case::allowed(Some("token"), "token", true)]
#[case::prefix(Some("token"), "os.environ/token", true)]
#[case::single_prefix(Some("token"), "os.environ/os.environ/token", false)]
#[case::denied(Some("token"), "other", false)]
#[case::unrestricted(None, "other", true)]
fn manager_read_requires_client_settings_and_allowlist_match(
    #[case] hosted: Option<&str>,
    #[case] name: &str,
    #[case] expected: bool,
) {
    let hosted_keys = hosted.map(|key| vec![key.to_owned()]);
    let enabled = SecretManagerReadState {
        client_exists: true,
        settings_exist: true,
        access_mode: SecretAccessMode::ReadOnly,
        hosted_keys: hosted_keys.as_deref(),
    };
    assert_eq!(manager_read_is_consulted(enabled, name), expected);
    assert!(!manager_read_is_consulted(
        SecretManagerReadState {
            hosted_keys: Some(&[]),
            ..enabled
        },
        name
    ));
}

#[rstest]
#[case::read_only((true, true, SecretAccessMode::ReadOnly), true)]
#[case::read_write((true, true, SecretAccessMode::ReadAndWrite), true)]
#[case::write_only((true, true, SecretAccessMode::WriteOnly), false)]
#[case::no_client((false, true, SecretAccessMode::ReadOnly), false)]
#[case::no_settings((true, false, SecretAccessMode::ReadOnly), false)]
fn read_gate_requires_client_settings_and_read_enabled_mode(
    #[case] setup: (bool, bool, SecretAccessMode),
    #[case] expected: bool,
) {
    let (client_exists, settings_exist, access_mode) = setup;
    let state = SecretManagerReadState {
        client_exists,
        settings_exist,
        access_mode,
        hosted_keys: None,
    };
    assert_eq!(manager_read_is_enabled(state), expected);
    assert_eq!(manager_read_is_consulted(state, "token"), expected);
}
