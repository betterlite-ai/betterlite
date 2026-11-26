use std::sync::PoisonError;

use betterlite_config::sections::settings::{DbRow, JsonValue};
use rstest::rstest;

use super::support::{fixture, json, values};
use crate::db_config_overlay::state::RuntimeEffect;

#[tokio::test]
async fn general_row_runs_the_effects_in_litellm_order() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    let row = json!({
        "alerting": ["slack"],
        "pass_through_endpoints": [{"path": "/a", "target": "https://a.test"}],
        "user_api_key_cache_max_size": 5,
        "maximum_spend_logs_retention_period": "1d",
        "user_url_validation": false
    });
    fixture.apply(&mut state, &row).await.unwrap();
    assert_eq!(
        fixture.journal.entries(),
        [
            "runtime:Alerting",
            "install",
            "cache",
            "scheduler",
            "runtime:UserUrlValidation"
        ]
    );
    assert_eq!(
        *fixture
            .cache
            .sizes
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
        [Some(5)]
    );
    assert_eq!(
        fixture.runtime.effects(),
        [
            RuntimeEffect::Alerting(vec![json!("slack")]),
            RuntimeEffect::UserUrlValidation(false)
        ]
    );
}

#[tokio::test]
async fn general_row_applies_its_values_to_the_store() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    fixture
        .apply(
            &mut state,
            &json!({"master_key": "sk-db", "max_parallel_requests": 3}),
        )
        .await
        .unwrap();
    assert_eq!(
        state.config.general_settings.get("master_key"),
        Some(&json!("sk-db"))
    );
    assert_eq!(
        state.config.general_settings.get("max_parallel_requests"),
        Some(&json!(3))
    );
}

#[tokio::test]
async fn general_row_refuses_remote_modules() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    let row = json!({"custom_auth": "s3://bucket/auth.py", "pass_through_endpoints": [{"path": "/m", "target": "gcs://bucket/m.handler"}]});
    fixture.apply(&mut state, &row).await.unwrap();
    assert_eq!(state.config.general_settings.get("custom_auth"), None);
    assert_eq!(
        fixture.installer.calls(),
        [vec![json!({"path": "/m", "target": null})]]
    );
}

#[tokio::test]
async fn general_row_sends_alerting_settings_only_from_the_database() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state
        .config
        .general_settings
        .load_yaml(values(&[("alert_types", json!(["budget_alerts"]))]));
    let row = json!({
        "alerting": "slack",
        "alert_types": ["llm_exceptions"],
        "alerting_args": {"daily_report_frequency": 60},
        "alert_to_webhook_url": {"budget_alerts": "https://hooks.test"}
    });
    fixture.apply(&mut state, &row).await.unwrap();
    assert_eq!(
        fixture.runtime.effects(),
        [
            RuntimeEffect::AlertingArgs(json!({"daily_report_frequency": 60})),
            RuntimeEffect::AlertToWebhookUrl(json!({"budget_alerts": "https://hooks.test"}))
        ]
    );
}

#[tokio::test]
async fn general_row_without_alerting_keys_sends_nothing() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state
        .config
        .general_settings
        .load_yaml(values(&[("alerting", json!(["slack"]))]));
    fixture
        .apply(&mut state, &json!({"alerting_args": null}))
        .await
        .unwrap();
    assert!(fixture.runtime.effects().is_empty());
}

#[tokio::test]
async fn general_row_replaces_the_plugins_from_the_database() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    let row = json!({"plugins": [{"name": "audit", "url": "https://plugin.test"}]});
    fixture.apply(&mut state, &row).await.unwrap();
    let plugins = fixture
        .plugins
        .plugins
        .read()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(plugins.keys().collect::<Vec<_>>(), ["audit"]);
}

#[tokio::test]
async fn general_row_leaves_file_plugins_alone() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state
        .config
        .general_settings
        .load_yaml(values(&[("plugins", json!([]))]));
    let row = json!({"plugins": [{"name": "audit", "url": "https://plugin.test"}]});
    fixture.apply(&mut state, &row).await.unwrap();
    assert!(
        fixture
            .plugins
            .plugins
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
}

#[rstest]
#[case::text_true(json!("TRUE"), json!(true))]
#[case::text_other(json!("yes"), json!(false))]
#[case::number(json!(1), json!(true))]
#[case::zero(json!(0), json!(false))]
#[case::boolean(json!(false), json!(false))]
#[tokio::test]
async fn general_row_coerces_its_flags(#[case] stored: JsonValue, #[case] coerced: JsonValue) {
    let mut fixture = fixture();
    let mut state = fixture.state();
    let row = values(&[
        ("store_prompts_in_spend_logs", stored.clone()),
        ("disable_auto_add_proxy_admin_to_teams", stored.clone()),
        ("apply_user_budget_to_team_keys", stored),
    ]);
    fixture
        .apply(&mut state, &JsonValue::Object(row.into_iter().collect()))
        .await
        .unwrap();
    let store = &state.config.general_settings;
    assert_eq!(store.get("store_prompts_in_spend_logs"), Some(&coerced));
    assert_eq!(
        store.get("disable_auto_add_proxy_admin_to_teams"),
        Some(&coerced)
    );
    assert_eq!(store.get("apply_user_budget_to_team_keys"), Some(&coerced));
}

#[tokio::test]
async fn general_row_leaves_file_owned_and_null_flags_alone() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state
        .config
        .general_settings
        .load_yaml(values(&[("store_prompts_in_spend_logs", json!("yes"))]));
    let row = json!({"store_prompts_in_spend_logs": true, "apply_user_budget_to_team_keys": null});
    fixture.apply(&mut state, &row).await.unwrap();
    let store = &state.config.general_settings;
    assert_eq!(
        store.get("store_prompts_in_spend_logs"),
        Some(&json!("yes"))
    );
    assert_eq!(store.get("apply_user_budget_to_team_keys"), None);
    assert_eq!(store.get("disable_auto_add_proxy_admin_to_teams"), None);
}

#[tokio::test]
async fn general_row_skips_a_flag_the_row_does_not_set() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state
        .config
        .general_settings
        .insert("store_prompts_in_spend_logs", json!("true"))
        .unwrap();
    fixture.apply(&mut state, &json!({})).await.unwrap();
    assert_eq!(
        state
            .config
            .general_settings
            .get("store_prompts_in_spend_logs"),
        Some(&json!("true"))
    );
}

#[rstest]
#[case::text_false(json!("false"), false)]
#[case::text_true(json!("True"), true)]
#[case::number(json!(2), true)]
#[tokio::test]
async fn general_row_sets_store_model_in_db(#[case] stored: JsonValue, #[case] enabled: bool) {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state.config.store_model_in_db = !enabled;
    let mut row = json!({});
    row["store_model_in_db"] = stored;
    fixture.apply(&mut state, &row).await.unwrap();
    assert_eq!(state.config.store_model_in_db, enabled);
    assert_eq!(
        state.config.general_settings.get("store_model_in_db"),
        Some(&JsonValue::Bool(enabled))
    );
}

#[tokio::test]
async fn general_row_keeps_a_file_owned_store_model_in_db_in_the_store() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state
        .config
        .general_settings
        .load_yaml(values(&[("store_model_in_db", json!("false"))]));
    fixture
        .apply(&mut state, &json!({"store_model_in_db": true}))
        .await
        .unwrap();
    assert!(!state.config.store_model_in_db);
    assert_eq!(
        state.config.general_settings.get("store_model_in_db"),
        Some(&json!("false"))
    );
}

#[rstest]
#[case::null(json!({"store_model_in_db": null}))]
#[case::absent(json!({}))]
#[tokio::test]
async fn general_row_without_store_model_in_db_keeps_the_flag(#[case] row: JsonValue) {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state
        .config
        .general_settings
        .insert("store_model_in_db", json!(false))
        .unwrap();
    fixture.apply(&mut state, &row).await.unwrap();
    assert!(state.config.store_model_in_db);
}

#[tokio::test]
async fn general_row_reads_the_cache_size_source_before_the_row() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    fixture
        .apply(&mut state, &json!({"user_api_key_cache_max_size": 9}))
        .await
        .unwrap();
    fixture.apply(&mut state, &json!({})).await.unwrap();
    assert_eq!(
        *fixture
            .cache
            .sizes
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
        [Some(9), None]
    );
    fixture.apply(&mut state, &json!({})).await.unwrap();
    assert_eq!(
        fixture
            .cache
            .sizes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        2
    );
}

#[tokio::test]
async fn general_row_reschedules_the_cleanup_job_once() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    let row = json!({"maximum_spend_logs_retention_period": "7d"});
    fixture.apply(&mut state, &row).await.unwrap();
    fixture.apply(&mut state, &row).await.unwrap();
    assert_eq!(fixture.scheduler.inserted, ["spend_log_cleanup_job"]);
    assert!(state.cleanup.last_attempt.is_some());
}

#[rstest]
#[case::text_true(json!("true"), Some(true))]
#[case::text_false(json!(" FALSE "), Some(false))]
#[case::text_other(json!("maybe"), None)]
#[case::boolean(json!(true), Some(true))]
#[case::zero(json!(0), Some(false))]
#[case::list(json!(["x"]), Some(true))]
#[case::null(json!(null), None)]
#[tokio::test]
async fn general_row_applies_user_url_validation(
    #[case] stored: JsonValue,
    #[case] expected: Option<bool>,
) {
    let mut fixture = fixture();
    let mut state = fixture.state();
    let mut row = json!({});
    row["user_url_validation"] = stored;
    fixture.apply(&mut state, &row).await.unwrap();
    let applied: Vec<RuntimeEffect> = expected
        .into_iter()
        .map(RuntimeEffect::UserUrlValidation)
        .collect();
    assert_eq!(fixture.runtime.effects(), applied);
}

#[tokio::test]
async fn general_row_applies_the_ssrf_host_lists() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    let row = json!({"user_url_allowed_hosts": ["a.test"], "provider_url_destination_allowed_hosts": null});
    fixture.apply(&mut state, &row).await.unwrap();
    assert_eq!(
        fixture.runtime.effects(),
        [
            RuntimeEffect::UserUrlAllowedHosts(json!(["a.test"])),
            RuntimeEffect::ProviderUrlAllowedHosts(json!(null))
        ]
    );
}

#[tokio::test]
async fn general_row_that_is_not_a_mapping_applies_an_empty_row() {
    let mut fixture = fixture();
    let mut state = fixture.state();
    state.config.general_settings.apply_db_row(
        DbRow::GeneralSettings,
        values(&[("master_key", json!("old"))]),
    );
    fixture
        .apply(&mut state, &json!(["not", "a", "mapping"]))
        .await
        .unwrap();
    assert_eq!(state.config.general_settings.get("master_key"), None);
    assert_eq!(
        state
            .config
            .general_settings
            .resolved()
            .keys()
            .collect::<Vec<_>>(),
        ["pass_through_endpoints"]
    );
}
