#![cfg(test)]

use std::collections::BTreeMap;

use super::{AuditSettingsError, Settings, decode_audit_settings, load_audit_settings};
use crate::loader::ConfigError;
use crate::sections::settings::JsonValue;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(source: &str, env: &BTreeMap<String, String>) -> Result<Settings, AuditSettingsError> {
    load_audit_settings(source, env)
}

fn settings(source: &str) -> Settings {
    load(source, &environment(&[])).expect("the settings load")
}

fn parsed(text: &str) -> JsonValue {
    serde_json::from_str::<JsonValue>(text).expect("the fixture is JSON")
}

#[test]
fn loads_audit_settings() {
    let absent = settings("");
    assert_eq!(absent.config.store_audit_logs, None);
    assert_eq!(absent.config.store_audit_logs_env, None);
    assert!(absent.config.audit_log_callbacks.is_empty());
    assert!(!absent.audit_log_callbacks_present);
    assert_eq!(absent.config.s3_audit_callback_params, None);

    let empty_list = settings("litellm_settings:\n  audit_log_callbacks: []\n");
    assert!(empty_list.audit_log_callbacks_present);
    assert!(empty_list.config.audit_log_callbacks.is_empty());

    let listed = settings(
        "litellm_settings:\n  audit_log_callbacks:\n    - s3_v2\n    - otel\n    - module.object\n",
    );
    assert!(listed.audit_log_callbacks_present);
    assert_eq!(
        listed.config.audit_log_callbacks,
        vec![
            "s3_v2".to_owned(),
            "otel".to_owned(),
            "module.object".to_owned()
        ]
    );

    let s3 = settings(
        "litellm_settings:\n  s3_audit_callback_params:\n    bucket_name: audit-bucket\n    prefix: audit/\n",
    );
    assert_eq!(
        s3.config.s3_audit_callback_params,
        Some(parsed(
            r#"{"bucket_name": "audit-bucket", "prefix": "audit/"}"#
        ))
    );
    let empty_s3 = settings("litellm_settings:\n  s3_audit_callback_params: {}\n");
    assert_eq!(empty_s3.config.s3_audit_callback_params, Some(parsed("{}")));
    let null_s3 = settings("litellm_settings:\n  s3_audit_callback_params: null\n");
    assert_eq!(null_s3.config.s3_audit_callback_params, None);
}

#[test]
fn loads_audit_settings_precedence() {
    let true_env = environment(&[("LITELLM_STORE_AUDIT_LOGS", "true")]);
    let false_env = environment(&[("LITELLM_STORE_AUDIT_LOGS", "false")]);

    let configured_false = load("litellm_settings:\n  store_audit_logs: false\n", &true_env)
        .expect("the settings load");
    assert_eq!(configured_false.config.store_audit_logs, Some(false));
    assert_eq!(configured_false.config.store_audit_logs_env, Some(true));
    assert!(!configured_false.config.is_audit_logging_enabled(None));
    assert!(configured_false.config.is_audit_logging_enabled(Some(true)));

    let configured_true = load("litellm_settings:\n  store_audit_logs: true\n", &false_env)
        .expect("the settings load");
    assert!(configured_true.config.is_audit_logging_enabled(None));
    assert!(!configured_true.config.is_audit_logging_enabled(Some(false)));

    let env_true = load("", &true_env).expect("the settings load");
    assert_eq!(env_true.config.store_audit_logs, None);
    assert!(env_true.config.is_audit_logging_enabled(None));

    let env_false = load("", &false_env).expect("the settings load");
    assert_eq!(env_false.config.store_audit_logs_env, Some(false));
    assert!(!env_false.config.is_audit_logging_enabled(None));

    let unset = settings("");
    assert!(unset.config.is_audit_logging_enabled(None));

    for text in ["yes", "", " TRUE "] {
        let env = environment(&[("LITELLM_STORE_AUDIT_LOGS", text)]);
        let resolved = load("", &env).expect("the settings load");
        if text == " TRUE " {
            assert_eq!(resolved.config.store_audit_logs_env, Some(true));
        } else {
            assert_eq!(resolved.config.store_audit_logs_env, None);
            assert!(resolved.config.is_audit_logging_enabled(None));
        }
    }
}

#[test]
fn loads_audit_settings_refuses_a_non_scalar_store_flag() {
    let error = load(
        "litellm_settings:\n  store_audit_logs:\n    enabled: true\n",
        &environment(&[]),
    )
    .err()
    .expect("an object is not a store flag");
    assert!(matches!(
        error,
        AuditSettingsError::Config(ConfigError::InvalidScalar { .. })
    ));
}

#[test]
fn decode_audit_settings_reads_the_same_keys() {
    let settings = decode_audit_settings(
        "litellm_settings:\n  store_audit_logs: true\n  audit_log_callbacks: [s3_v2]\n",
    )
    .expect("the settings decode");
    assert_eq!(settings.config.store_audit_logs, Some(true));
    assert!(settings.audit_log_callbacks_present);
    assert_eq!(
        settings.config.audit_log_callbacks,
        vec!["s3_v2".to_owned()]
    );
}
