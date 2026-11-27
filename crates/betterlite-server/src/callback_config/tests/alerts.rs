use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use betterlite_config::sections::settings::JsonValue;
use secrecy::SecretString;

use super::support::{Harness, get_request, internal_key, json, master_key, object, webhook};

const CALLBACKS_URI: &str = "/get/config/callbacks";

fn alert_entry<'entry>(body: &'entry JsonValue, name: &str) -> &'entry JsonValue {
    let entries = body["alerts"].as_array().expect("alert entries");
    entries
        .iter()
        .find(|entry| entry["name"] == name)
        .expect("the alert entry")
}

fn alert_names(body: &JsonValue) -> Vec<String> {
    body["alerts"]
        .as_array()
        .expect("alert entries")
        .iter()
        .map(|entry| entry["name"].as_str().expect("a name").to_owned())
        .collect()
}

#[tokio::test]
async fn masks_alert_credentials_for_admins_and_redacts_them_for_viewers() {
    let config = object(&[(
        "environment_variables",
        json(
            r#"{"SMTP_PASSWORD": "smtp-password-abcd", "SMTP_USERNAME": "mailer", "MS_TEAMS_WEBHOOK_URL": "https://teams.example/hook"}"#,
        ),
    )]);
    let harness = Harness::new(config);
    let (_status, admin_body) = harness
        .call(get_request(CALLBACKS_URI), Some(master_key()))
        .await;
    let email = &alert_entry(&admin_body, "email")["variables"];
    assert_eq!(email["SMTP_USERNAME"], json(r#""mailer""#));
    assert_eq!(email["SMTP_PORT"], json(r#""587""#));
    assert_eq!(email["SMTP_TLS"], json(r#""True""#));
    assert_eq!(email["SMTP_HOST"], json("null"));
    let password = email["SMTP_PASSWORD"].as_str().expect("a string");
    assert_ne!(password, "smtp-password-abcd");
    assert!(password.starts_with("smtp"));
    assert!(password.ends_with("abcd"));
    assert!(password.contains('*'));
    let teams = &alert_entry(&admin_body, "ms_teams")["variables"];
    let teams_url = teams["MS_TEAMS_WEBHOOK_URL"].as_str().expect("a string");
    assert_ne!(teams_url, "https://teams.example/hook");
    assert!(teams_url.starts_with("http"));
    assert!(teams_url.ends_with("hook"));
    let (_status, viewer_body) = harness
        .call(get_request(CALLBACKS_URI), Some(internal_key()))
        .await;
    let viewer_email = &alert_entry(&viewer_body, "email")["variables"];
    assert_eq!(viewer_email["SMTP_PASSWORD"], json(r#""REDACTED""#));
    assert_eq!(viewer_email["SMTP_USERNAME"], json(r#""REDACTED""#));
    assert_eq!(viewer_email["SMTP_PORT"], json(r#""587""#));
    assert_eq!(
        alert_entry(&viewer_body, "ms_teams")["variables"]["MS_TEAMS_WEBHOOK_URL"],
        json(r#""REDACTED""#)
    );
}

#[tokio::test]
async fn resolves_alert_variables_from_the_stored_environment_then_the_process() {
    let config = object(&[
        (
            "environment_variables",
            json(r#"{"SMTP_HOST": "", "SMTP_PORT": "2525", "SMTP_SENDER_EMAIL": null}"#),
        ),
        ("general_settings", json(r#"{"alerting": ["email"]}"#)),
    ]);
    let harness = Harness::new(config).with_environment(&[
        ("SMTP_HOST", "env-host"),
        ("SMTP_PORT", "587"),
        ("SMTP_SENDER_EMAIL", "env@example.com"),
        ("SMTP_TLS", "False"),
    ]);
    let (_status, body) = harness
        .call(get_request(CALLBACKS_URI), Some(master_key()))
        .await;
    let variables = &alert_entry(&body, "email")["variables"];
    assert_eq!(variables["SMTP_HOST"], json(r#""""#));
    assert_eq!(variables["SMTP_PORT"], json(r#""2525""#));
    assert_eq!(variables["SMTP_SENDER_EMAIL"], json(r#""env@example.com""#));
    assert_eq!(variables["SMTP_TLS"], json(r#""False""#));
    assert_eq!(variables["EMAIL_LOGO_URL"], json("null"));
    assert_eq!(variables["SMTP_USERNAME"], json("null"));
}

#[tokio::test]
async fn lists_slack_alerts_only_when_slack_is_configured() {
    let without = Harness::new(BTreeMap::new());
    let (_status, body) = without
        .call(get_request(CALLBACKS_URI), Some(master_key()))
        .await;
    assert_eq!(alert_names(&body), vec!["email", "ms_teams"]);
    assert_eq!(body["active_alerting_destinations"], json("[]"));
    let mut webhooks = BTreeMap::new();
    webhooks.insert(
        AlertType::LlmExceptions,
        webhook("https://hooks.example/slack"),
    );
    let with = Harness::new(object(&[(
        "general_settings",
        json(r#"{"alerting": ["slack", "email"]}"#),
    )]))
    .with_alerting(vec![AlertType::LlmExceptions], webhooks);
    let (_status, body) = with
        .call(get_request(CALLBACKS_URI), Some(master_key()))
        .await;
    assert_eq!(alert_names(&body), vec!["slack", "email", "ms_teams"]);
    assert_eq!(
        body["active_alerting_destinations"],
        json(r#"["slack", "email"]"#)
    );
    let slack = alert_entry(&body, "slack");
    assert_eq!(slack["active_alerts"], json(r#"["llm_exceptions"]"#));
    assert_eq!(
        slack["alerts_to_webhook"],
        json(r#"{"llm_exceptions": "https://hooks.example/slack"}"#)
    );
}

#[tokio::test]
async fn redacts_the_slack_webhook_map_for_a_viewer() {
    let mut webhooks = BTreeMap::new();
    webhooks.insert(
        AlertType::LlmExceptions,
        webhook("https://hooks.example/slack"),
    );
    webhooks.insert(
        AlertType::BudgetAlerts,
        WebhookUrl::Multiple(vec![
            SecretString::from("https://hooks.example/one".to_owned()),
            SecretString::from("https://hooks.example/two".to_owned()),
        ]),
    );
    let harness = Harness::new(object(&[(
        "general_settings",
        json(r#"{"alerting": ["slack"]}"#),
    )]))
    .with_alerting(vec![AlertType::LlmExceptions], webhooks);
    let (_status, admin_body) = harness
        .call(get_request(CALLBACKS_URI), Some(master_key()))
        .await;
    assert_eq!(
        alert_entry(&admin_body, "slack")["alerts_to_webhook"],
        json(
            r#"{"budget_alerts": ["https://hooks.example/one", "https://hooks.example/two"], "llm_exceptions": "https://hooks.example/slack"}"#
        )
    );
    let (_status, viewer_body) = harness
        .call(get_request(CALLBACKS_URI), Some(internal_key()))
        .await;
    assert_eq!(
        alert_entry(&viewer_body, "slack")["alerts_to_webhook"],
        json(r#"{"budget_alerts": "REDACTED", "llm_exceptions": "REDACTED"}"#)
    );
    assert_eq!(
        alert_entry(&viewer_body, "slack")["variables"]["SLACK_WEBHOOK_URL"],
        json("null")
    );
}

#[tokio::test]
async fn rejects_an_alerting_value_that_is_not_a_list() {
    let harness = Harness::new(object(&[(
        "general_settings",
        json(r#"{"alerting": "slack"}"#),
    )]));
    let (status, body) = harness
        .call(get_request(CALLBACKS_URI), Some(master_key()))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json(
            r#"{"error": {"message": "Authentication Error, general_settings.alerting must be a list of strings", "type": "auth_error", "param": "None", "code": "400"}}"#
        )
    );
}
