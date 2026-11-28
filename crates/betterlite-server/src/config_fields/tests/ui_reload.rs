#![cfg(test)]

use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use betterlite_config::sections::settings::{JsonValue, Section};
use serde_json::json;

use super::{
    ADMIN, Overlay, bad_request, body_text, caller, delete, fixtures, object, state_with_general,
    update,
};
use crate::audit_events::enums::AuditAction;
use crate::config_fields::ports::ConfigFieldServices;
use crate::config_fields::records::ConfigFieldUpdateResponse;
use crate::config_fields::write::{delete_config_general_settings, update_config_general_settings};
use crate::config_overlay_port::ConfigOverlayPortError;

#[tokio::test]
async fn ui_field_reload_persist_sets_the_litellm_attribute_and_saves_the_section() {
    let mut fakes = fixtures();
    fakes.overlay = Overlay::with_values(object(
        r#"{"litellm_settings": {"openai_system_messages_first": true}}"#,
    ));
    let mut state = state_with_general("{}");
    let written = update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update("budget_rollover", json!(true)),
    )
    .await
    .expect("the UI persist");
    match written {
        ConfigFieldUpdateResponse::Ui(body) => {
            assert_eq!(body.message, "Field budget_rollover updated");
            assert_eq!(body.status, "success");
        }
        ConfigFieldUpdateResponse::Row(row) => panic!("a UI answer, got {row:?}"),
    }
    assert_eq!(fakes.runtime.ui_value("budget_rollover"), json!(true));
    let writes = fakes.store.section_writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].section, Section::LitellmSettings);
    assert_eq!(writes[0].changed.get("budget_rollover"), Some(&json!(true)));
    let audit = fakes.audit_rows();
    assert_eq!(audit[0].action, AuditAction::Updated);
    assert_eq!(audit[0].object_id, "budget_rollover");
}

#[tokio::test]
async fn ui_field_reload_reset_sets_the_default_and_pops_the_stored_key() {
    let mut fakes = fixtures();
    fakes.overlay = Overlay::with_values(object(
        r#"{"litellm_settings": {"max_ui_session_budget": 5.0}}"#,
    ));
    let mut state = state_with_general("{}");
    state.replace_snapshot(&object(
        r#"{"litellm_settings": {"max_ui_session_budget": 5.0}}"#,
    ));
    let written = delete_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &delete("max_ui_session_budget"),
    )
    .await
    .expect("the UI reset");
    match written {
        ConfigFieldUpdateResponse::Ui(body) => {
            assert_eq!(body.message, "Field max_ui_session_budget reset");
        }
        ConfigFieldUpdateResponse::Row(row) => panic!("a UI answer, got {row:?}"),
    }
    assert_eq!(fakes.runtime.ui_value("max_ui_session_budget"), json!(1.0));
    let writes = fakes.store.section_writes();
    assert_eq!(writes.len(), 1);
    assert!(writes[0].removed.contains("max_ui_session_budget"));
    assert_eq!(fakes.audit_rows()[0].action, AuditAction::Deleted);
}

#[tokio::test]
async fn ui_field_reload_reset_without_a_section_keeps_it_absent() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    delete_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &delete("budget_rollover"),
    )
    .await
    .expect("the UI reset");
    assert_eq!(fakes.runtime.ui_value("budget_rollover"), json!(false));
    assert!(fakes.store.section_writes().is_empty());
    assert_eq!(fakes.audit_rows()[0].action, AuditAction::Deleted);
}

#[tokio::test]
async fn ui_field_reload_validates_before_it_reloads() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    let refusal = update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update("budget_exceeded_throttle_percentage", json!(2.0)),
    )
    .await
    .expect_err("a fraction above one");
    assert_eq!(
        bad_request(refusal),
        "budget_exceeded_throttle_percentage must be a number in (0, 1] or empty"
    );
    assert!(fakes.store.section_writes().is_empty());
    assert!(fakes.audit_rows().is_empty());
    assert_eq!(
        fakes
            .runtime
            .ui_value("budget_exceeded_throttle_percentage"),
        JsonValue::Null
    );
}

#[tokio::test]
async fn ui_field_reload_reload_failure_leaves_the_runtime_untouched() {
    let mut fakes = fixtures();
    fakes.overlay.reload_error = Some(ConfigOverlayPortError::Reload);
    let mut state = state_with_general("{}");
    let refusal = update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update("budget_rollover", json!(true)),
    )
    .await
    .expect_err("the reload failed");
    let response = refusal.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(fakes.runtime.ui_value("budget_rollover"), JsonValue::Null);
    assert!(fakes.store.section_writes().is_empty());
    assert!(fakes.audit_rows().is_empty());
}

#[tokio::test]
async fn ui_field_reload_database_unavailable_answers_503() {
    let mut fakes = fixtures();
    fakes.overlay.reload_error = Some(ConfigOverlayPortError::DatabaseUnavailable);
    let mut state = state_with_general("{}");
    let refusal = delete_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &delete("budget_rollover"),
    )
    .await
    .expect_err("the reload is unavailable");
    let response = refusal.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(body_text(response).await.contains("no_db_connection"));
    assert_eq!(fakes.runtime.ui_value("budget_rollover"), JsonValue::Null);
    assert!(fakes.audit_rows().is_empty());
}
