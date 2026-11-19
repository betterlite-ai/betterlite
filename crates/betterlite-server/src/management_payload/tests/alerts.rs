#![cfg(test)]

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_models::model_registration::JsonObject;
use serde_json::json;

use crate::management_payload::alerts::{
    ManagementAlertError, displayed_event_name, management_event_alert,
    send_management_endpoint_alert,
};
use crate::management_payload::tests::support::{
    FakeTransportError, RecordingAlertTransport, json_object, json_value, key_fixture,
};

const MAPPINGS: [(&str, AlertType, &str); 9] = [
    (
        "generate_key_fn",
        AlertType::NewVirtualKeyCreated,
        "New Virtual Key Created",
    ),
    (
        "update_key_fn",
        AlertType::VirtualKeyUpdated,
        "Virtual Key Updated",
    ),
    (
        "delete_key_fn",
        AlertType::VirtualKeyDeleted,
        "Virtual Key Deleted",
    ),
    ("new_team", AlertType::NewTeamCreated, "New Team Created"),
    ("update_team", AlertType::TeamUpdated, "Team Updated"),
    ("delete_team", AlertType::TeamDeleted, "Team Deleted"),
    (
        "new_user",
        AlertType::NewInternalUserCreated,
        "New Internal User Created",
    ),
    (
        "user_update",
        AlertType::InternalUserUpdated,
        "Internal User Updated",
    ),
    (
        "delete_user",
        AlertType::InternalUserDeleted,
        "Internal User Deleted",
    ),
];

#[test]
fn nine_alert_mappings() {
    for (function_name, alert_type, event_name) in MAPPINGS {
        assert_eq!(management_event_alert(function_name), Some(alert_type));
        assert_eq!(displayed_event_name(alert_type), event_name);
    }
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

#[tokio::test]
async fn unknown_actor_defaults() {
    let transport = RecordingAlertTransport::default();
    let absent = key_fixture("{}");
    let empty = key_fixture(r#"{"user_id": "", "key_alias": null}"#);
    let nulled = key_fixture(r#"{"user_id": null, "user_role": null}"#);
    for key in [&absent, &empty, &nulled] {
        let sent =
            send_management_endpoint_alert(Some(&transport), "new_team", JsonObject::new(), key)
                .await;
        assert!(sent.is_ok());
    }
    let sent = transport.alerts.entries();
    assert_eq!(sent.len(), 3);
    for alert in &sent {
        assert_eq!(alert.created_by_user_id, "Unknown");
        assert_eq!(alert.created_by_user_role, "Unknown");
        assert_eq!(alert.created_by_key_alias, None);
    }
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
    let sent = transport.alerts.entries();
    assert_eq!(sent.len(), 1);
    let alert = &sent[0];
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
