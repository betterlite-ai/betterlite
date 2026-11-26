use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::settings::settings_store::config_ownership_message;
use betterlite_config::sections::settings::{DbRow, JsonValue, Section};
use betterlite_db::postgres_store::errors::DbError;

use super::support::{
    RecordingRuntime, database, failing_database, json, overlay_db_rows, sealed, state, values,
};
use crate::config_boot::test_support::capture_logs;
use crate::db_config_overlay::error::OverlayError;
use crate::db_config_overlay::state::RuntimeEffect;

#[tokio::test]
async fn rows_apply_each_section_to_its_store() {
    let runtime = Arc::new(RecordingRuntime::default());
    let mut overlay = state(true, &runtime);
    let rows = database(&[
        ("general_settings", json!({"master_key": "sk-db"})),
        ("router_settings", json!({"num_retries": 3})),
        ("litellm_settings", json!({"drop_params": true})),
    ]);
    overlay_db_rows(&mut overlay, &rows, &mut BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(
        overlay.config.general_settings.get("master_key"),
        Some(&json!("sk-db"))
    );
    assert_eq!(
        overlay.config.router_settings.get("num_retries"),
        Some(&json!(3))
    );
    assert_eq!(
        overlay.config.litellm_settings.get("drop_params"),
        Some(&json!(true))
    );
    assert!(runtime.effects().is_empty());
}

#[tokio::test]
async fn rows_skip_missing_and_null_rows() {
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    overlay.config.general_settings.apply_db_row(
        DbRow::GeneralSettings,
        values(&[("master_key", json!("sk-old"))]),
    );
    overlay
        .config
        .router_settings
        .apply_db_row(DbRow::RouterSettings, values(&[("num_retries", json!(1))]));
    let rows = database(&[("general_settings", json!(null))]);
    overlay_db_rows(&mut overlay, &rows, &mut BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(
        overlay.config.general_settings.get("master_key"),
        Some(&json!("sk-old"))
    );
    assert_eq!(
        overlay.config.router_settings.get("num_retries"),
        Some(&json!(1))
    );
}

#[tokio::test]
async fn rows_apply_a_row_that_is_not_a_mapping_as_empty() {
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    overlay
        .config
        .router_settings
        .apply_db_row(DbRow::RouterSettings, values(&[("num_retries", json!(1))]));
    let rows = database(&[("router_settings", json!(["num_retries"]))]);
    overlay_db_rows(&mut overlay, &rows, &mut BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(overlay.config.router_settings.get("num_retries"), None);
}

#[tokio::test]
async fn rows_export_decrypted_environment_values_under_both_names() {
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    let mut stored = json!({"api_key": null, "PLAIN": "kept", "skipped": null});
    stored["api_key"] = JsonValue::String(sealed("sk-secret"));
    let rows = database(&[("environment_variables", stored)]);
    let mut env = BTreeMap::new();
    overlay_db_rows(&mut overlay, &rows, &mut env)
        .await
        .unwrap();
    assert_eq!(
        env,
        BTreeMap::from([
            ("API_KEY".to_owned(), "sk-secret".to_owned()),
            ("PLAIN".to_owned(), "kept".to_owned()),
            ("api_key".to_owned(), "sk-secret".to_owned()),
        ])
    );
    let store = &overlay.config.environment_variables;
    assert_eq!(store.get("api_key"), Some(&json!("sk-secret")));
    assert_eq!(store.get("API_KEY"), Some(&json!("sk-secret")));
    assert_eq!(store.get("PLAIN"), Some(&json!("kept")));
    assert_eq!(store.get("skipped"), None);
}

#[tokio::test]
async fn rows_ignore_an_environment_row_that_is_not_a_mapping() {
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    overlay.config.environment_variables.apply_db_row(
        DbRow::EnvironmentVariables,
        values(&[("OLD", json!("old"))]),
    );
    let rows = database(&[("environment_variables", json!("OLD=new"))]);
    let mut env = BTreeMap::new();
    overlay_db_rows(&mut overlay, &rows, &mut env)
        .await
        .unwrap();
    assert!(env.is_empty());
    assert_eq!(overlay.config.environment_variables.get("OLD"), None);
}

#[tokio::test]
async fn rows_send_the_safe_litellm_overrides_with_the_store_value() {
    let runtime = Arc::new(RecordingRuntime::default());
    let mut overlay = state(true, &runtime);
    overlay
        .config
        .litellm_settings
        .load_yaml(values(&[("budget_rollover", json!(false))]));
    let rows = database(&[(
        "litellm_settings",
        json!({
            "turn_off_message_logging": true,
            "budget_rollover": true,
            "max_ui_session_budget": null,
            "num_retries": 3
        }),
    )]);
    overlay_db_rows(&mut overlay, &rows, &mut BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(
        runtime.effects(),
        [
            RuntimeEffect::LitellmSetting {
                key: "budget_rollover",
                value: json!(false)
            },
            RuntimeEffect::LitellmSetting {
                key: "turn_off_message_logging",
                value: json!(true)
            },
        ]
    );
}

#[tokio::test]
async fn rows_scrub_remote_modules_before_the_store() {
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    let rows = database(&[
        (
            "general_settings",
            json!({"custom_auth": "s3://bucket/auth.py"}),
        ),
        (
            "litellm_settings",
            json!({"callbacks": ["gcs://bucket/cb.py", "langfuse"]}),
        ),
        (
            "router_settings",
            json!({"custom_auth": "s3://bucket/auth.py"}),
        ),
    ]);
    overlay_db_rows(&mut overlay, &rows, &mut BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(overlay.config.general_settings.get("custom_auth"), None);
    assert_eq!(
        overlay.config.litellm_settings.get("callbacks"),
        Some(&json!(["langfuse"]))
    );
    assert_eq!(
        overlay.config.router_settings.get("custom_auth"),
        Some(&json!("s3://bucket/auth.py"))
    );
}

#[tokio::test]
async fn rows_warn_about_a_shadowed_key_once_per_shadowing() {
    let (logs, _guard) = capture_logs();
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    overlay
        .config
        .general_settings
        .load_yaml(values(&[("master_key", json!("sk-file"))]));
    let shadowing = database(&[("general_settings", json!({"master_key": "sk-db"}))]);
    let agreeing = database(&[("general_settings", json!({"master_key": "sk-file"}))]);
    let message = config_ownership_message(Section::GeneralSettings, "master_key", true);
    overlay_db_rows(&mut overlay, &shadowing, &mut BTreeMap::new())
        .await
        .unwrap();
    overlay_db_rows(&mut overlay, &shadowing, &mut BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(logs.contents().matches(&message).count(), 1);
    overlay_db_rows(&mut overlay, &agreeing, &mut BTreeMap::new())
        .await
        .unwrap();
    overlay_db_rows(&mut overlay, &shadowing, &mut BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(logs.contents().matches(&message).count(), 2);
}

#[tokio::test]
async fn rows_warn_about_shadowed_keys_in_section_name_order() {
    let (logs, _guard) = capture_logs();
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    overlay
        .config
        .router_settings
        .load_yaml(values(&[("num_retries", json!(1))]));
    overlay
        .config
        .general_settings
        .load_yaml(values(&[("master_key", json!("sk-file"))]));
    let rows = database(&[
        ("router_settings", json!({"num_retries": 2})),
        ("general_settings", json!({"master_key": "sk-db"})),
    ]);
    overlay_db_rows(&mut overlay, &rows, &mut BTreeMap::new())
        .await
        .unwrap();
    let contents = logs.contents();
    let general = contents
        .find(&config_ownership_message(
            Section::GeneralSettings,
            "master_key",
            true,
        ))
        .unwrap();
    let router = contents
        .find(&config_ownership_message(
            Section::RouterSettings,
            "num_retries",
            true,
        ))
        .unwrap();
    assert!(general < router);
}

#[tokio::test]
async fn rows_report_a_failed_read() {
    let mut overlay = state(true, &Arc::new(RecordingRuntime::default()));
    let rows = failing_database(|| DbError::NoDbConnected);
    let error = overlay_db_rows(&mut overlay, &rows, &mut BTreeMap::new())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        OverlayError::Database {
            source: DbError::NoDbConnected
        }
    ));
}
