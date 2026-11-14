use std::time::Duration;

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use secrecy::SecretString;
use tokio::task::yield_now;
use tokio::time::{advance, timeout};

use super::super::destination::{AlertDestination, SLACK_WEBHOOK_URL_ENV};
use super::*;
use crate::alert_batching::alert_headers;
use crate::alert_settings::records::{AlertQueueItem, AlertText};
use crate::alert_teams::MS_TEAMS_WEBHOOK_URL_ENV;
use crate::batch::logger::DEFAULT_FLUSH_INTERVAL_SECONDS;

#[derive(Debug, serde::Deserialize)]
struct TextPayload {
    text: String,
}

fn slack_fixture_with_batch(batch_size: usize) -> Fixture {
    fixture(
        &[(SLACK_WEBHOOK_URL_ENV, "https://hooks/slack")],
        batch_size,
    )
}

async fn settle(delivery: &FakeDelivery, expected: usize) {
    let mut attempts = 0_u32;
    while delivery.request_count() < expected && attempts < 32 {
        yield_now().await;
        attempts = attempts.saturating_add(1);
    }
}

async fn stop(tasks: &TaskTracker) {
    tasks.close();
    advance(Duration::from_secs(DEFAULT_FLUSH_INTERVAL_SECONDS)).await;
    timeout(Duration::from_secs(15), tasks.wait())
        .await
        .expect("the flush task stops after the tracker closes");
}

async fn assert_no_periodic_flush(alerting: Option<Vec<AlertDestination>>) {
    let fixture = slack_fixture();
    let _queued = fixture.delivery.queue.enqueue(AlertQueueItem {
        url: SecretString::from("https://hooks/queued"),
        headers: alert_headers(),
        payload: AlertText {
            text: "queued".to_owned(),
        },
        alert_type: AlertType::LlmExceptions,
        format: None,
    });
    let dispatcher = fixture.dispatcher(alerting, vec![AlertType::LlmExceptions]);
    let _sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    yield_now().await;
    advance(Duration::from_secs(60)).await;
    assert_eq!(fixture.delivery.delivery.request_count(), 0);
    assert_eq!(fixture.delivery.queue.pending().len(), 1);
}

#[tokio::test]
async fn reaching_the_batch_size_flushes_the_queue() {
    let fixture = slack_fixture_with_batch(1);
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let _sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(fixture.delivery.queue.pending().is_empty());
    assert_eq!(fixture.delivery.delivery.request_count(), 1);
    let requests = fixture.delivery.delivery.take_requests();
    let payload: TextPayload = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        payload.text,
        "Alert type: `llm_exceptions`\nLevel: `High`\nTimestamp: `12:00:00`\n\nMessage: boom"
    );
    assert_eq!(requests[0].url.as_str(), "https://hooks/slack");
}

#[tokio::test]
async fn a_queue_below_the_batch_size_is_not_flushed() {
    let fixture = slack_fixture_with_batch(2);
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let _sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert_eq!(fixture.delivery.queue.pending().len(), 1);
    assert_eq!(fixture.delivery.delivery.request_count(), 0);
}

#[tokio::test]
async fn a_webhook_url_list_flushes_when_the_last_url_reaches_the_batch_size() {
    let mut fixture = fixture(&[], 2);
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
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(fixture.delivery.queue.pending().is_empty());
    assert_eq!(fixture.delivery.delivery.request_count(), 2);
}

#[tokio::test]
async fn a_teams_only_configuration_flushes_when_the_queue_is_full() {
    let fixture = fixture(&[(MS_TEAMS_WEBHOOK_URL_ENV, "https://hooks/teams")], 1);
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::MsTeams]),
        vec![AlertType::LlmExceptions],
    );
    let _sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(fixture.delivery.queue.pending().is_empty());
    assert_eq!(fixture.delivery.delivery.request_count(), 1);
    let requests = fixture.delivery.delivery.take_requests();
    let body = String::from_utf8(requests[0].body.clone()).unwrap();
    assert!(body.contains("AdaptiveCard"));
}

#[tokio::test(start_paused = true)]
async fn the_first_send_starts_the_periodic_flush() {
    let fixture = slack_fixture();
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    assert_eq!(fixture.delivery.delivery.request_count(), 0);
    yield_now().await;
    advance(Duration::from_secs(DEFAULT_FLUSH_INTERVAL_SECONDS)).await;
    settle(&fixture.delivery.delivery, 1).await;
    assert_eq!(fixture.delivery.delivery.request_count(), 1);
    stop(&fixture.tasks).await;
}

#[tokio::test(start_paused = true)]
async fn absent_or_empty_alerting_does_not_start_the_periodic_flush() {
    assert_no_periodic_flush(None).await;
    assert_no_periodic_flush(Some(Vec::new())).await;
}
