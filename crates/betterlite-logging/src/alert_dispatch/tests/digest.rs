use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::destination::{AlertDestination, SLACK_WEBHOOK_URL_ENV};
use super::super::{Alert, AlertLevel, SendAlertError};
use super::*;
use crate::alert_teams::MS_TEAMS_WEBHOOK_URL_ENV;

#[tokio::test]
async fn teams_alerts_are_enqueued_before_slack_digest_admission() {
    let fixture = digest_fixture(
        &[
            (SLACK_WEBHOOK_URL_ENV, "https://hooks/slack"),
            (MS_TEAMS_WEBHOOK_URL_ENV, "https://hooks/teams"),
        ],
        512,
        &[AlertType::LlmExceptions],
    );
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::MsTeams, AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    let pending = fixture.delivery.queue.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].url.expose_secret(), "https://hooks/teams");
    assert_eq!(pending[0].format.as_deref(), Some("ms_teams"));
    let entries = fixture.digest_entries();
    assert_eq!(entries.len(), 1);
    let entry = entries
        .get("llm_exceptions::")
        .expect("the digest bucket exists");
    match &entry.webhook_url {
        WebhookUrl::Single(url) => assert_eq!(url.expose_secret(), "https://hooks/slack"),
        WebhookUrl::Multiple(_) => panic!("the slack webhook is a single URL"),
    }
}

#[tokio::test]
async fn digest_mode_records_the_first_message_and_suppresses_the_slack_item() {
    let fixture = digest_fixture(
        &[(SLACK_WEBHOOK_URL_ENV, "https://hooks/slack")],
        512,
        &[AlertType::LlmExceptions],
    );
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("first", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    assert!(fixture.delivery.queue.pending().is_empty());
    let entries = fixture.digest_entries();
    assert_eq!(entries.len(), 1);
    let entry = entries
        .get("llm_exceptions::")
        .expect("the digest bucket exists");
    assert_eq!(entry.first_message, "first");
    assert_eq!(entry.level, "High");
    assert_eq!(entry.count, 1);
    assert_eq!(entry.start_time, time(12, 0, 0));
    assert_eq!(entry.last_time, time(12, 0, 0));
}

#[tokio::test]
async fn a_repeated_digest_alert_updates_only_the_count_and_last_time() {
    let fixture = digest_fixture(
        &[(SLACK_WEBHOOK_URL_ENV, "https://hooks/slack")],
        512,
        &[AlertType::LlmExceptions],
    );
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    assert!(
        dispatcher
            .send_alert(fixture.alert("first", AlertType::LlmExceptions))
            .await
            .is_ok()
    );
    fixture.clock.set(time(12, 0, 7));
    assert!(
        dispatcher
            .send_alert(fixture.alert("second", AlertType::LlmExceptions))
            .await
            .is_ok()
    );
    let entries = fixture.digest_entries();
    assert_eq!(entries.len(), 1);
    let entry = entries
        .get("llm_exceptions::")
        .expect("the digest bucket exists");
    assert_eq!(entry.count, 2);
    assert_eq!(entry.first_message, "first");
    assert_eq!(entry.start_time, time(12, 0, 0));
    assert_eq!(entry.last_time, time(12, 0, 7));
}

#[tokio::test]
async fn digest_buckets_are_keyed_by_type_model_and_api_base() {
    let fixture = digest_fixture(
        &[(SLACK_WEBHOOK_URL_ENV, "https://hooks/slack")],
        512,
        &[AlertType::LlmExceptions],
    );
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let metadata = IndexMap::new();
    let kwargs = IndexMap::new();
    let mut alert = Alert {
        message: "one",
        level: AlertLevel::High,
        alert_type: AlertType::LlmExceptions,
        alerting_metadata: &metadata,
        user_info: None,
        request_model: Some("gpt-4"),
        api_base: Some("https://api.example"),
        kwargs: &kwargs,
    };
    assert!(dispatcher.send_alert(alert).await.is_ok());
    alert.message = "two";
    alert.api_base = Some("https://other.example");
    assert!(dispatcher.send_alert(alert).await.is_ok());
    let entries = fixture.digest_entries();
    assert_eq!(entries.len(), 2);
    assert!(entries.contains_key("llm_exceptions:gpt-4:https://api.example"));
    assert!(entries.contains_key("llm_exceptions:gpt-4:https://other.example"));
}

#[tokio::test]
async fn a_digest_entry_keeps_a_webhook_url_list_as_given() {
    let mut fixture = digest_fixture(&[], 512, &[AlertType::LlmExceptions]);
    fixture.webhooks.insert(
        AlertType::LlmExceptions,
        WebhookUrl::Multiple(vec![
            SecretString::from("https://hooks/a"),
            SecretString::from("https://hooks/b"),
        ]),
    );
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let _sent = dispatcher
        .send_alert(fixture.alert("first", AlertType::LlmExceptions))
        .await;
    let entries = fixture.digest_entries();
    let entry = entries
        .get("llm_exceptions::")
        .expect("the digest bucket exists");
    match &entry.webhook_url {
        WebhookUrl::Multiple(urls) => assert_eq!(urls.len(), 2),
        WebhookUrl::Single(_) => panic!("the configured list is stored as a list"),
    }
}

#[tokio::test]
async fn digest_mode_is_enabled_per_alert_type() {
    let fixture = digest_fixture(
        &[(SLACK_WEBHOOK_URL_ENV, "https://hooks/slack")],
        512,
        &[AlertType::LlmExceptions],
    );
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions, AlertType::LlmTooSlow],
    );
    let _digested = dispatcher
        .send_alert(fixture.alert("digested", AlertType::LlmExceptions))
        .await;
    assert!(fixture.delivery.queue.pending().is_empty());
    let _immediate = dispatcher
        .send_alert(fixture.alert("immediate", AlertType::LlmTooSlow))
        .await;
    let pending = fixture.delivery.queue.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].alert_type, AlertType::LlmTooSlow);
}

#[tokio::test]
async fn a_digested_alert_without_a_webhook_url_raises_before_recording() {
    let fixture = digest_fixture(&[], 512, &[AlertType::LlmExceptions]);
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("first", AlertType::LlmExceptions))
        .await;
    assert!(matches!(sent, Err(SendAlertError::MissingWebhookUrl(_))));
    assert!(fixture.digest_entries().is_empty());
}
