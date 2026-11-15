#![cfg(test)]
#![cfg(test)]

use std::collections::BTreeMap;

use serde_json::json;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::audit_events::environment_flag;

fn load(source: &str, lookup: BTreeMap<String, String>) -> crate::sections::audit_events::Settings {
    ConfigLoader { lookup }
        .load(source)
        .expect("audit config should load")
        .sections
        .audit_events
}

#[test]
fn audit_explicit_false_wins() {
    let settings = load(
        "litellm_settings:\n  store_audit_logs: true\n",
        BTreeMap::from([("LITELLM_STORE_AUDIT_LOGS".to_owned(), "true".to_owned())]),
    );
    assert!(!settings.is_audit_logging_enabled(Some(false)));
    assert!(settings.is_audit_logging_enabled(Some(true)));
    assert!(settings.is_audit_logging_enabled(None));
}

#[test]
fn audit_setting_over_env() {
    let disabled = load(
        "litellm_settings:\n  store_audit_logs: false\n",
        BTreeMap::from([("LITELLM_STORE_AUDIT_LOGS".to_owned(), "true".to_owned())]),
    );
    assert!(!disabled.is_audit_logging_enabled(None));
    let enabled = load(
        "litellm_settings:\n  store_audit_logs: true\n",
        BTreeMap::from([("LITELLM_STORE_AUDIT_LOGS".to_owned(), "false".to_owned())]),
    );
    assert!(enabled.is_audit_logging_enabled(None));
}

#[test]
fn audit_env_bool_and_unknown() {
    let unset = load("{}", BTreeMap::new());
    assert_eq!(unset.store_audit_logs_env, None);
    let padded = load(
        "{}",
        BTreeMap::from([("LITELLM_STORE_AUDIT_LOGS".to_owned(), " TrUe ".to_owned())]),
    );
    assert_eq!(padded.store_audit_logs_env, Some(true));
    let disabled = load(
        "{}",
        BTreeMap::from([("LITELLM_STORE_AUDIT_LOGS".to_owned(), "false".to_owned())]),
    );
    assert_eq!(disabled.store_audit_logs_env, Some(false));
    let unknown = load(
        "{}",
        BTreeMap::from([("LITELLM_STORE_AUDIT_LOGS".to_owned(), " yes ".to_owned())]),
    );
    assert_eq!(unknown.store_audit_logs_env, None);
    assert!(unknown.is_audit_logging_enabled(None));
}

#[test]
fn audit_environment_flag_matches_get_secret_bool() {
    assert_eq!(environment_flag(None), None);
    assert_eq!(environment_flag(Some(" TRUE ")), Some(true));
    assert_eq!(environment_flag(Some("false")), Some(false));
    assert_eq!(environment_flag(Some("yes")), None);
    assert_eq!(environment_flag(Some("1")), None);
    assert_eq!(environment_flag(Some("")), None);
}

#[test]
fn audit_default_enabled() {
    let settings = load("{}", BTreeMap::new());
    assert_eq!(settings.store_audit_logs, None);
    assert!(settings.is_audit_logging_enabled(None));
    assert!(!settings.is_audit_logging_enabled(Some(false)));
}

#[test]
fn audit_environment_reference_is_resolved_with_the_secret_conversion() {
    let settings = load(
        "litellm_settings:\n  store_audit_logs: os.environ/AUDIT_FLAG\n",
        BTreeMap::from([("AUDIT_FLAG".to_owned(), "FALSE".to_owned())]),
    );
    assert_eq!(settings.store_audit_logs, Some(false));
    let unknown = load(
        "litellm_settings:\n  store_audit_logs: os.environ/AUDIT_FLAG\n",
        BTreeMap::from([("AUDIT_FLAG".to_owned(), "enabled".to_owned())]),
    );
    assert_eq!(unknown.store_audit_logs, None);
    assert!(unknown.is_audit_logging_enabled(None));
}

#[test]
fn audit_missing_environment_is_named() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("litellm_settings:\n  store_audit_logs: os.environ/AUDIT_FLAG\n")
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
        if variable == "AUDIT_FLAG" && path == "litellm_settings.store_audit_logs"
    ));
}

#[test]
fn audit_wrong_setting_type_is_rejected() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("litellm_settings:\n  store_audit_logs: 1\n")
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("litellm_settings.store_audit_logs")
    );
}

#[test]
fn audit_callbacks_and_s3_params_are_loaded() {
    let settings = load(
        "litellm_settings:\n  audit_log_callbacks:\n    - s3_v2\n    - otel\n  s3_audit_callback_params:\n    bucket_name: audit-bucket\n",
        BTreeMap::new(),
    );
    assert_eq!(settings.audit_log_callbacks, ["s3_v2", "otel"]);
    assert_eq!(
        settings.s3_audit_callback_params,
        Some(json!({"bucket_name": "audit-bucket"}))
    );
    let default = load("{}", BTreeMap::new());
    assert!(default.audit_log_callbacks.is_empty());
    assert_eq!(default.s3_audit_callback_params, None);
}

#[test]
fn audit_empty_s3_params_select_the_override() {
    let settings = load(
        "litellm_settings:\n  s3_audit_callback_params: {}\n",
        BTreeMap::new(),
    );
    assert_eq!(settings.s3_audit_callback_params, Some(json!({})));
}
