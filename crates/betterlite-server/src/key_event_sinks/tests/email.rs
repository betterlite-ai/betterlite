use std::sync::{Arc, Mutex};

use betterlite_logging::budget_email::event::{EntityType, WebhookEvent, WebhookEventType};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret;
use tokio::sync::Notify;
use tokio::task::yield_now;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::support::{GatedMailer, Logs, lock};
use crate::key_event_sinks::email::{CreatedKey, LegacyEmail};
use crate::key_event_sinks::error::EmailError;

fn alerting(destinations: &[&str]) -> Vec<String> {
    destinations
        .iter()
        .map(|destination| (*destination).to_owned())
        .collect()
}

async fn created(
    destinations: &[&str],
    response: &str,
) -> (Result<(), EmailError>, Vec<WebhookEvent>) {
    let mailer = Arc::new(GatedMailer::default());
    let tasks = TaskTracker::new();
    let configured = alerting(destinations);
    let email = LegacyEmail {
        alerting: &configured,
        mailer: Arc::<GatedMailer>::clone(&mailer),
        tasks: &tasks,
    };
    let sent = email.send_key_created_email(serde_json::from_str::<CreatedKey>(response).unwrap());
    tasks.close();
    tasks.wait().await;
    let events = lock(&mailer.events).clone();
    (sent, events)
}

#[rstest]
#[case::absent(&[])]
#[case::other_destinations(&["slack", "webhook"])]
#[tokio::test]
async fn email_disabled_debug_skip(#[case] destinations: &[&str]) {
    let logs = Logs::default();
    let _logging = set_default(tracing_subscriber::registry().with(logs.clone()));
    let (sent, events) = created(destinations, r#"{"token": "hashed"}"#).await;
    sent.unwrap();
    assert!(events.is_empty());
    let tasks = TaskTracker::new();
    let configured = alerting(destinations);
    let email = LegacyEmail {
        alerting: &configured,
        mailer: Arc::new(GatedMailer::default()),
        tasks: &tasks,
    };
    email.send_key_rotated_email();
    assert!(tasks.is_empty());
    let expected = [
        (
            Level::DEBUG,
            "Email sending not enabled, skipping key created email".to_owned(),
        ),
        (
            Level::DEBUG,
            "Email sending not enabled, skipping key rotated email".to_owned(),
        ),
    ];
    assert_eq!(logs.lines(), expected);
}

#[rstest]
#[case::only_email(&["email"], 1)]
#[case::among_others(&["slack", "email"], 1)]
#[case::capitalised(&["Email"], 0)]
#[case::plural(&["emails"], 0)]
#[case::padded(&[" email"], 0)]
#[case::empty_name(&[""], 0)]
#[tokio::test]
async fn email_membership_shape_semantics(#[case] destinations: &[&str], #[case] sends: usize) {
    let (sent, events) = created(destinations, "{}").await;
    sent.unwrap();
    assert_eq!(events.len(), sends);
}

fn token(event: &WebhookEvent) -> Option<&str> {
    event.token.as_ref().map(ExposeSecret::expose_secret)
}

#[tokio::test]
async fn legacy_creation_event_all_defaults() {
    let (sent, events) = created(&["email"], "{}").await;
    sent.unwrap();
    let [event] = events.as_slice() else {
        panic!("one event, got {}", events.len())
    };
    assert_eq!(event.event, WebhookEventType::KeyCreated);
    assert_eq!(event.event_group, EntityType::Key);
    assert_eq!(event.event_message, "API Key Created");
    assert_eq!(token(event), Some(""));
    assert_eq!(event.spend, Decimal::ZERO);
    assert_eq!(event.max_budget, Some(Decimal::ZERO));
    assert_eq!(event.user_id, None);
    assert_eq!(event.team_id.as_deref(), Some("Default Team"));
    assert_eq!(event.key_alias, None);
    assert_eq!(
        (event.soft_budget, &event.user_email, &event.team_alias),
        (None, &None, &None)
    );
}

#[tokio::test]
async fn legacy_creation_explicit_nulls() {
    let response = r#"{"token": null, "spend": 1.5, "max_budget": null, "user_id": null, "team_id": null, "key_alias": null}"#;
    let (sent, events) = created(&["email"], response).await;
    sent.unwrap();
    let [event] = events.as_slice() else {
        panic!("one event, got {}", events.len())
    };
    assert_eq!(token(event), None);
    assert_eq!(event.spend, Decimal::new(15, 1));
    assert_eq!(
        (
            event.max_budget,
            &event.user_id,
            &event.team_id,
            &event.key_alias
        ),
        (None, &None, &None, &None)
    );
    let (null_spend, unsent) = created(&["email"], r#"{"spend": null}"#).await;
    assert_eq!(null_spend, Err(EmailError::NullSpend));
    assert!(unsent.is_empty());
}

#[tokio::test]
async fn legacy_email_uses_token_not_key() {
    let response = r#"{"key": "sk-new-raw-key", "token": "hashed-token", "spend": 2, "max_budget": 10, "user_id": "u1", "team_id": "t1", "key_alias": "alias"}"#;
    let (sent, events) = created(&["email"], response).await;
    sent.unwrap();
    let [event] = events.as_slice() else {
        panic!("one event, got {}", events.len())
    };
    assert_eq!(token(event), Some("hashed-token"));
    assert!(!format!("{event:?}").contains("sk-new-raw-key"));
    assert_eq!(
        (event.spend, event.max_budget),
        (Decimal::from(2_i32), Some(Decimal::from(10_i32)))
    );
    assert_eq!(
        (
            event.user_id.as_deref(),
            event.team_id.as_deref(),
            event.key_alias.as_deref()
        ),
        (Some("u1"), Some("t1"), Some("alias"))
    );
}

#[tokio::test]
async fn creation_schedules_without_waiting() {
    let mailer = Arc::new(GatedMailer {
        gate: Notify::new(),
        held: true,
        events: Mutex::default(),
    });
    let tasks = TaskTracker::new();
    let destinations = alerting(&["email"]);
    let email = LegacyEmail {
        alerting: &destinations,
        mailer: Arc::<GatedMailer>::clone(&mailer),
        tasks: &tasks,
    };
    email.send_key_created_email(CreatedKey::default()).unwrap();
    assert_eq!(tasks.len(), 1);
    yield_now().await;
    assert!(lock(&mailer.events).is_empty());
    mailer.gate.notify_one();
    tasks.close();
    tasks.wait().await;
    assert_eq!(lock(&mailer.events).len(), 1);
}

#[tokio::test]
async fn rotation_email_unavailable_debug_skip() {
    let logs = Logs::default();
    let _logging = set_default(tracing_subscriber::registry().with(logs.clone()));
    let mailer = Arc::new(GatedMailer::default());
    let tasks = TaskTracker::new();
    let destinations = alerting(&["email"]);
    let email = LegacyEmail {
        alerting: &destinations,
        mailer: Arc::<GatedMailer>::clone(&mailer),
        tasks: &tasks,
    };
    email.send_key_rotated_email();
    assert!(tasks.is_empty());
    assert!(lock(&mailer.events).is_empty());
    let expected = [(
        Level::DEBUG,
        "Enterprise package not installed, skipping key rotated email".to_owned(),
    )];
    assert_eq!(logs.lines(), expected);
}
