use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_config::sections::settings::{JsonValue, Section};

use crate::config_overlay_port::ConfigOverlayPortError;
use crate::settings_services::error::SettingsUpdateError;
use crate::settings_services::keys::SettingKey;

use super::support::{Harness, SETTING, as_json, json, record, values};

async fn error_body(error: SettingsUpdateError) -> (StatusCode, JsonValue) {
    let response: Response = error.into_response();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice::<JsonValue>(&bytes).unwrap();
    (status, body)
}

#[test]
fn setting_keys_name_the_litellm_settings_blocks() {
    assert_eq!(
        SettingKey::DefaultInternalUserParams.as_str(),
        "default_internal_user_params"
    );
    assert_eq!(
        SettingKey::DefaultTeamParams.as_str(),
        "default_team_params"
    );
    assert_eq!(
        SettingKey::McpSemanticToolFilter.as_str(),
        "mcp_semantic_tool_filter"
    );
    assert_eq!(
        SettingKey::WebsearchInterceptionParams.as_str(),
        "websearch_interception_params"
    );
    assert_eq!(SettingKey::McpToolSearch.as_str(), "mcp_tool_search");
}

#[tokio::test]
async fn publishes_the_pruned_settings_and_answers_the_caller_message() {
    let mut harness = Harness::new(values(&[(
        "litellm_settings",
        json!({"other_setting": {"enabled": true}}),
    )]));
    let settings = record(values(&[
        ("user_role", json!("internal_user_viewer")),
        ("max_budget", json!(null)),
        ("models", json!(["a"])),
    ]));
    let response = harness.update(&settings).await.unwrap();
    let stored = json!({"user_role": "internal_user_viewer", "models": ["a"]});
    assert_eq!(
        as_json(&response),
        json!({
            "message": "Internal user settings updated successfully",
            "status": "success",
            "settings": {"user_role": "internal_user_viewer", "models": ["a"]},
        })
    );
    let published = harness.published();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].0, SETTING);
    assert_eq!(published[0].1, stored);
    let section = harness.stored(Section::LitellmSettings).unwrap();
    assert_eq!(
        section.get("other_setting"),
        Some(&json!({"enabled": true}))
    );
    assert_eq!(section.get("default_internal_user_params"), Some(&stored));
}

#[tokio::test]
async fn loads_before_publishing_and_audits_only_after_the_save() {
    let mut harness = Harness::new(values(&[]));
    let settings = record(values(&[("user_role", json!("internal_user_viewer"))]));
    harness.update(&settings).await.unwrap();
    let recorded = harness.recording();
    let reload = recorded
        .iter()
        .position(|event| *event == "reload")
        .unwrap();
    let publish = recorded
        .iter()
        .position(|event| *event == "publish")
        .unwrap();
    let save = recorded.iter().position(|event| *event == "save").unwrap();
    let audit = recorded.iter().position(|event| *event == "audit").unwrap();
    assert!(reload < publish);
    assert!(publish < save);
    assert!(save < audit);
    let audited = harness.audited();
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0].setting, SETTING);
    assert_eq!(audited[0].before_value, None);
    assert_eq!(
        audited[0].after_value,
        json!({"user_role": "internal_user_viewer"})
    );
    assert_eq!(
        audited[0].admin.user_id.as_deref(),
        Some("admin@example.com")
    );
}

#[tokio::test]
async fn the_audit_receives_the_replaced_block() {
    let mut harness = Harness::new(values(&[(
        "litellm_settings",
        json!({"default_internal_user_params": {"user_role": "old_role"}}),
    )]));
    let settings = record(values(&[("user_role", json!("new_role"))]));
    harness.update(&settings).await.unwrap();
    let audited = harness.audited();
    assert_eq!(audited.len(), 1);
    assert_eq!(
        audited[0].before_value,
        Some(json!({"user_role": "old_role"}))
    );
    assert_eq!(audited[0].after_value, json!({"user_role": "new_role"}));
}

#[tokio::test]
async fn null_values_are_pruned_recursively_and_list_positions_stay() {
    let mut harness = Harness::new(values(&[]));
    let settings = record(values(&[
        ("max_budget", json!(null)),
        ("models", json!(["a", null])),
        ("teams", json!([{"team_id": "t", "max_budget": null}])),
    ]));
    let response = harness.update(&settings).await.unwrap();
    let stored = json!({"models": ["a", null], "teams": [{"team_id": "t"}]});
    assert_eq!(
        as_json(&response),
        json!({
            "message": "Internal user settings updated successfully",
            "status": "success",
            "settings": {"models": ["a", null], "teams": [{"team_id": "t"}]},
        })
    );
    assert_eq!(harness.published()[0].1, stored);
}

#[tokio::test]
async fn null_litellm_settings_is_replaced_with_the_selected_block() {
    let mut harness = Harness::new(values(&[("litellm_settings", json!(null))]));
    let settings = record(values(&[("user_role", json!("internal_user_viewer"))]));
    harness.update(&settings).await.unwrap();
    let section = harness.stored(Section::LitellmSettings).unwrap();
    assert_eq!(
        section.get("default_internal_user_params"),
        Some(&json!({"user_role": "internal_user_viewer"}))
    );
}

#[tokio::test]
async fn storage_disabled_refuses_before_reading_the_config() {
    let mut harness = Harness::disabled(values(&[]));
    let settings = record(values(&[("max_budget", json!(null))]));
    let error = harness
        .update(&settings)
        .await
        .expect_err("storage is disabled");
    assert!(matches!(&error, SettingsUpdateError::StorageDisabled));
    assert!(harness.recording().is_empty());
    assert!(harness.published().is_empty());
    assert!(harness.audited().is_empty());
    assert_eq!(harness.stored(Section::LitellmSettings), None);
}

#[tokio::test]
async fn storage_disabled_answers_the_pinned_500_detail() {
    let (status, body) = error_body(SettingsUpdateError::StorageDisabled).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json!({
            "detail": {
                "error": "Set `'STORE_MODEL_IN_DB='True'` in your env to enable this feature."
            }
        })
    );
}

#[tokio::test]
async fn reload_database_unavailable_answers_503_and_leaves_nothing_behind() {
    let mut harness = Harness::failing_reload(ConfigOverlayPortError::DatabaseUnavailable);
    let settings = record(values(&[("user_role", json!("internal_user_viewer"))]));
    let error = harness
        .update(&settings)
        .await
        .expect_err("the config database is unavailable");
    assert_eq!(harness.recording(), vec!["reload"]);
    assert!(harness.published().is_empty());
    assert!(harness.audited().is_empty());
    assert_eq!(harness.stored(Section::LitellmSettings), None);
    let (status, body) = error_body(error).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body,
        json!({
            "error": {
                "message": "Service Unavailable, the authentication database is temporarily unreachable. Please retry shortly.",
                "type": "no_db_connection",
                "param": "None",
                "code": "503",
            }
        })
    );
}

#[tokio::test]
async fn reload_failure_answers_500_and_leaves_nothing_behind() {
    let mut harness = Harness::failing_reload(ConfigOverlayPortError::Reload);
    let settings = record(values(&[("user_role", json!("internal_user_viewer"))]));
    let error = harness
        .update(&settings)
        .await
        .expect_err("the config could not be reloaded");
    assert_eq!(harness.recording(), vec!["reload"]);
    assert!(harness.published().is_empty());
    assert!(harness.audited().is_empty());
    assert_eq!(harness.stored(Section::LitellmSettings), None);
    let (status, body) = error_body(error).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json!({
            "error": {
                "message": "Internal server error",
                "type": "internal_server_error",
            }
        })
    );
}

#[tokio::test]
async fn a_non_object_litellm_settings_is_refused_before_publishing() {
    let mut harness = Harness::new(values(&[("litellm_settings", json!([1]))]));
    let settings = record(values(&[("user_role", json!("internal_user_viewer"))]));
    let error = harness
        .update(&settings)
        .await
        .expect_err("litellm_settings is not an object");
    assert!(matches!(&error, SettingsUpdateError::Config(_)));
    assert_eq!(harness.recording(), vec!["reload"]);
    assert!(harness.published().is_empty());
    assert!(harness.audited().is_empty());
    assert_eq!(harness.stored(Section::LitellmSettings), None);
    let (status, body) = error_body(error).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json!({"error": {"message": "Internal server error", "type": "internal_server_error"}})
    );
}

#[tokio::test]
async fn failed_save_keeps_the_published_value_and_skips_the_audit() {
    let mut harness = Harness::failing_database(values(&[]));
    let settings = record(values(&[("user_role", json!("internal_user_viewer"))]));
    let error = harness
        .update(&settings)
        .await
        .expect_err("the config database write failed");
    assert!(matches!(&error, SettingsUpdateError::Save(_)));
    let published = harness.published();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].1, json!({"user_role": "internal_user_viewer"}));
    assert!(harness.audited().is_empty());
    assert!(harness.recording().contains(&"save"));
    assert_eq!(harness.stored(Section::LitellmSettings), None);
}

#[tokio::test]
async fn config_owned_setting_answers_400_after_publishing() {
    let mut harness = Harness::new(values(&[]));
    harness.state_mut().litellm_settings.load_yaml(values(&[(
        "default_internal_user_params",
        json!({"user_role": "file_role"}),
    )]));
    let settings = record(values(&[("user_role", json!("internal_user_viewer"))]));
    let error = harness
        .update(&settings)
        .await
        .expect_err("the config file owns the setting");
    assert!(matches!(&error, SettingsUpdateError::Save(_)));
    assert_eq!(harness.published().len(), 1);
    assert!(harness.audited().is_empty());
    assert_eq!(harness.stored(Section::LitellmSettings), None);
    let (status, body) = error_body(error).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json!({
            "detail": {
                "error": "litellm_settings key 'default_internal_user_params' is set in the config file and cannot be changed here.",
                "keys": ["default_internal_user_params"],
                "section": "litellm_settings",
                "stored_database_values_ignored": [],
                "resolution": "edit /etc/betterlite/config.yaml to change it, or remove it from the file to let the database own it",
            }
        })
    );
}
