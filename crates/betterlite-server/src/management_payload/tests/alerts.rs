#![cfg(test)]

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use serde_json::json;

use crate::alerting::model_alerts::key_event::KeyEventAlerts;
use crate::management_payload::alerts::{
    ManagementAlertError, displayed_event_name, management_event_alert,
    send_management_endpoint_alert,
};
use crate::management_payload::tests::support::{
    FakeTransportError, RecordingAlertTransport, RecordingModelAlertSender, json_object,
    json_value, key_fixture,
};

#[rstest]
#[case::generate_key(
    "generate_key_fn",
    AlertType::NewVirtualKeyCreated,
    "New Virtual Key Created"
)]
#[case::update_key("update_key_fn", AlertType::VirtualKeyUpdated, "Virtual Key Updated")]
#[case::delete_key("delete_key_fn", AlertType::VirtualKeyDeleted, "Virtual Key Deleted")]
#[case::new_team("new_team", AlertType::NewTeamCreated, "New Team Created")]
#[case::update_team("update_team", AlertType::TeamUpdated, "Team Updated")]
#[case::delete_team("delete_team", AlertType::TeamDeleted, "Team Deleted")]
#[case::new_user(
    "new_user",
    AlertType::NewInternalUserCreated,
    "New Internal User Created"
)]
#[case::user_update("user_update", AlertType::InternalUserUpdated, "Internal User Updated")]
#[case::delete_user("delete_user", AlertType::InternalUserDeleted, "Internal User Deleted")]
fn nine_alert_mappings(
    #[case] function_name: &str,
    #[case] alert_type: AlertType,
    #[case] event_name: &str,
) {
    assert_eq!(management_event_alert(function_name), Some(alert_type));
    assert_eq!(displayed_event_name(alert_type), event_name);
}

#[test]
fn unknown_function_has_no_alert() {
    assert_eq!(management_event_alert("generate_key"), None);
}

#[tokio::test]
async fn absent_logger_sink_unknown_function() {
    let key = key_fixture(r#"{"user_id": "u1"}"#);
    let transport = RecordingAlertTransport::default();
    let skipped = send_management_endpoint_alert::<RecordingAlertTransport>(
        None,
        "generate_key_fn",
        JsonObject::new(),
        &key,
    )
    .await;
    assert!(skipped.is_ok());
    let unknown =
        send_management_endpoint_alert(Some(&transport), "generate_key", JsonObject::new(), &key)
            .await;
    assert!(unknown.is_ok());
    assert!(transport.alerts.entries().is_empty());
}

#[rstest]
#[case::absent("{}", "Unknown", None)]
#[case::nulled(r#"{"user_id": null, "user_role": null}"#, "Unknown", None)]
// an explicit empty user id and alias are used as given, not read as unknown
#[case::explicit_empty(r#"{"user_id": "", "key_alias": ""}"#, "", Some(""))]
#[tokio::test]
async fn unknown_actor_defaults(
    #[case] fields: &str,
    #[case] expected_id: &str,
    #[case] expected_alias: Option<&str>,
) {
    let transport = RecordingAlertTransport::default();
    let key = key_fixture(fields);
    let sent =
        send_management_endpoint_alert(Some(&transport), "new_team", JsonObject::new(), &key).await;
    assert!(sent.is_ok());
    let alerts = transport.alerts.entries();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].created_by_user_id, expected_id);
    assert_eq!(alerts[0].created_by_user_role, "Unknown");
    assert_eq!(alerts[0].created_by_key_alias.as_deref(), expected_alias);
}

#[tokio::test]
async fn alert_kwargs_and_alias_preserved() {
    let transport = RecordingAlertTransport::default();
    let key =
        key_fixture(r#"{"user_id": "admin", "user_role": "proxy_admin", "key_alias": "ops-key"}"#);
    let kwargs: JsonObject = json_object(json!({
        "api_key": "sk-secret",
        "nested": {"password": "p@ss"},
        "user_id": "someone"
    }));
    let sent =
        send_management_endpoint_alert(Some(&transport), "update_key_fn", kwargs.clone(), &key)
            .await;
    assert!(sent.is_ok());
    let alerts = transport.alerts.entries();
    assert_eq!(alerts.len(), 1);
    let alert = &alerts[0];
    assert_eq!(alert.created_by_user_id, "admin");
    assert_eq!(alert.created_by_user_role, "proxy_admin");
    assert_eq!(alert.created_by_key_alias.as_deref(), Some("ops-key"));
    // the alert carries the caller's kwargs untouched, credentials included
    assert_eq!(alert.request_kwargs, kwargs);
    assert_eq!(
        alert.request_kwargs.get("api_key"),
        Some(&json_value(json!("sk-secret")))
    );
    assert_eq!(alert.alert_type, AlertType::VirtualKeyUpdated);
    assert_eq!(alert.event_name, "Virtual Key Updated");
}

#[tokio::test]
async fn slack_alert_reaches_the_sender() {
    let sender = RecordingModelAlertSender::default();
    let transport = KeyEventAlerts {
        sender: sender.clone(),
    };
    let key = key_fixture(r#"{"user_id": "admin", "key_alias": "ops-key"}"#);
    let kwargs: JsonObject = json_object(json!({"api_key": "sk-secret"}));
    send_management_endpoint_alert(Some(&transport), "generate_key_fn", kwargs, &key)
        .await
        .expect("the slack alert is sent");
    let sent = sender.alerts.entries();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].alert_type, AlertType::NewVirtualKeyCreated);
    assert!(sent[0].message.contains("`New Virtual Key Created`"));
    assert!(sent[0].message.contains("created_by_user_id: `admin`"));
}

#[tokio::test]
async fn alert_failure_propagates() {
    let transport = RecordingAlertTransport::failing();
    let key = key_fixture(r#"{"user_id": "admin"}"#);
    let result =
        send_management_endpoint_alert(Some(&transport), "delete_key_fn", JsonObject::new(), &key)
            .await;
    assert!(matches!(
        result,
        Err(ManagementAlertError::Alert(FakeTransportError))
    ));
    assert!(transport.alerts.entries().is_empty());
}
