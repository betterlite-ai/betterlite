use std::collections::BTreeMap;

use super::support::{capture_logs, cipher, env_values, sealed};
use crate::db_row_decrypt::env::decrypt_and_export_env;
use crate::db_row_decrypt::warnings::Warnings;

const STDIO_WARNING: &str = "Ignoring LITELLM_ENABLE_MCP_STDIO stored in the database. Set it in the proxy's environment instead";

#[test]
fn skips_the_stdio_flag_with_one_warning_per_process() {
    let (logs, _guard) = capture_logs();
    let cipher = cipher();
    let warnings = Warnings::default();
    let mut environment = BTreeMap::new();
    let sealed = sealed(&cipher, "true");
    let values = env_values(&format!(
        r#"{{"LITELLM_ENABLE_MCP_STDIO":"{sealed}","litellm_enable_mcp_stdio":"{sealed}"}}"#
    ));
    let first = decrypt_and_export_env(&values, false, &cipher, &mut environment, &warnings);
    assert_eq!(logs.contents().matches(STDIO_WARNING).count(), 1);
    let second = decrypt_and_export_env(&values, false, &cipher, &mut environment, &warnings);
    assert!(first.is_empty());
    assert!(second.is_empty());
    assert!(environment.is_empty());
    assert_eq!(logs.contents().matches(STDIO_WARNING).count(), 1);
}

#[test]
fn exports_and_returns_decrypted_values() {
    let cipher = cipher();
    let values = env_values(&format!(
        r#"{{"API_KEY":"{}","API_BASE":"https://example.test"}}"#,
        sealed(&cipher, "sk-db-secret")
    ));
    let mut environment = BTreeMap::from([("EXISTING".to_owned(), "kept".to_owned())]);
    let exported = decrypt_and_export_env(
        &values,
        false,
        &cipher,
        &mut environment,
        &Warnings::default(),
    );
    assert_eq!(
        exported.get("API_KEY").map(String::as_str),
        Some("sk-db-secret")
    );
    assert_eq!(
        environment.get("API_KEY").map(String::as_str),
        Some("sk-db-secret")
    );
    assert_eq!(
        environment.get("EXISTING").map(String::as_str),
        Some("kept")
    );
    assert_eq!(exported.len(), 1);
}

#[test]
fn drops_values_that_decrypt_to_nothing() {
    let cipher = cipher();
    let values = env_values(r#"{"BROKEN":"not-a-ciphertext"}"#);
    let mut environment = BTreeMap::new();
    let exported = decrypt_and_export_env(
        &values,
        false,
        &cipher,
        &mut environment,
        &Warnings::default(),
    );
    assert!(exported.is_empty());
    assert!(environment.is_empty());
}

#[test]
fn keeps_the_stored_text_when_keep_original_is_set() {
    let cipher = cipher();
    let values = env_values(r#"{"BROKEN":"not-a-ciphertext"}"#);
    let mut environment = BTreeMap::new();
    let exported = decrypt_and_export_env(
        &values,
        true,
        &cipher,
        &mut environment,
        &Warnings::default(),
    );
    assert_eq!(
        exported.get("BROKEN").map(String::as_str),
        Some("not-a-ciphertext")
    );
    assert_eq!(
        environment.get("BROKEN").map(String::as_str),
        Some("not-a-ciphertext")
    );
}

#[test]
fn a_non_text_value_logs_and_does_not_stop_the_rest() {
    let (logs, _guard) = capture_logs();
    let cipher = cipher();
    let values = env_values(&format!(
        r#"{{"PORT":6379,"RATIO":1.5,"API_KEY":"{}"}}"#,
        sealed(&cipher, "sk-db-secret")
    ));
    let mut environment = BTreeMap::new();
    let exported = decrypt_and_export_env(
        &values,
        false,
        &cipher,
        &mut environment,
        &Warnings::default(),
    );
    assert_eq!(
        exported.get("API_KEY").map(String::as_str),
        Some("sk-db-secret")
    );
    assert_eq!(exported.get("PORT"), None);
    assert_eq!(environment.get("PORT"), None);
    assert_eq!(exported.get("RATIO"), None);
    assert!(
        logs.contents()
            .contains("Error setting env variable: PORT - str expected, not int")
    );
    assert!(
        logs.contents()
            .contains("Error setting env variable: RATIO - str expected, not float")
    );
}

#[test]
fn a_null_value_is_skipped_without_an_error() {
    let (logs, _guard) = capture_logs();
    let cipher = cipher();
    let values = env_values(r#"{"ABSENT":null}"#);
    let mut environment = BTreeMap::new();
    let exported = decrypt_and_export_env(
        &values,
        false,
        &cipher,
        &mut environment,
        &Warnings::default(),
    );
    assert!(exported.is_empty());
    assert!(environment.is_empty());
    assert!(!logs.contents().contains("Error setting env variable"));
}
