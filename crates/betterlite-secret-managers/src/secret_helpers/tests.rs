#![cfg(test)]

use super::{
    SecretManagerReadState, manager_read_is_consulted, manager_read_is_enabled,
    normalize_nonempty_secret_str, read_boolean_text,
};

use crate::secret_types::SecretAccessMode;

#[test]
fn parses_only_true_and_false_after_trim_and_case_fold() {
    assert_eq!(read_boolean_text(Some(" TRUE ")), Some(true));
    assert_eq!(read_boolean_text(Some("False")), Some(false));
    assert_eq!(read_boolean_text(Some("yes")), None);
    assert_eq!(read_boolean_text(None), None);
}

#[test]
fn normalizes_empty_secret_values() {
    assert_eq!(
        normalize_nonempty_secret_str(Some("  secret \n")),
        Some(String::from("secret"))
    );
    assert_eq!(normalize_nonempty_secret_str(Some("  ")), None);
    assert_eq!(normalize_nonempty_secret_str(None), None);
}

#[test]
fn manager_read_requires_client_settings_and_allowlist_match() {
    let hosted = vec![String::from("token")];
    let enabled = SecretManagerReadState {
        client_exists: true,
        settings_exist: true,
        access_mode: SecretAccessMode::ReadOnly,
        hosted_keys: Some(&hosted),
    };
    assert!(manager_read_is_consulted(enabled, "token"));
    assert!(manager_read_is_consulted(enabled, "os.environ/token"));
    assert!(manager_read_is_consulted(
        SecretManagerReadState {
            access_mode: SecretAccessMode::ReadAndWrite,
            hosted_keys: None,
            ..enabled
        },
        "other"
    ));
    assert!(!manager_read_is_consulted(
        SecretManagerReadState {
            access_mode: SecretAccessMode::WriteOnly,
            hosted_keys: None,
            ..enabled
        },
        "token"
    ));
    assert!(!manager_read_is_consulted(
        SecretManagerReadState {
            client_exists: false,
            hosted_keys: None,
            ..enabled
        },
        "token"
    ));
    assert!(!manager_read_is_consulted(enabled, "other"));
}

#[test]
fn read_gate_requires_client_settings_and_read_enabled_mode() {
    let enabled = SecretManagerReadState {
        client_exists: true,
        settings_exist: true,
        access_mode: SecretAccessMode::ReadOnly,
        hosted_keys: None,
    };
    assert!(manager_read_is_enabled(enabled));
    assert!(manager_read_is_enabled(SecretManagerReadState {
        access_mode: SecretAccessMode::ReadAndWrite,
        ..enabled
    }));
    assert!(!manager_read_is_enabled(SecretManagerReadState {
        access_mode: SecretAccessMode::WriteOnly,
        ..enabled
    }));
    assert!(!manager_read_is_enabled(SecretManagerReadState {
        client_exists: false,
        ..enabled
    }));
    assert!(!manager_read_is_enabled(SecretManagerReadState {
        settings_exist: false,
        ..enabled
    }));
}
