use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::destination::{
    ALERTING_WEBHOOK_URL_ENV, AlertDestination, MissingWebhookUrl, SLACK_WEBHOOK_URL_ENV,
};
use super::super::{Alert, AlertLevel, SendAlertError};
use super::{SinkCall, budget_event, fixture, slack_fixture};
use crate::alert_teams::MS_TEAMS_WEBHOOK_URL_ENV;
use crate::budget_email::event::WebhookEvent;

fn alert_with_event<'alert>(
    alert_type: AlertType,
    event: &'alert WebhookEvent,
    metadata: &'alert IndexMap<String, Json>,
    kwargs: &'alert IndexMap<String, Json>,
) -> Alert<'alert> {
    Alert {
        message: "budget crossed",
        level: AlertLevel::High,
        alert_type,
        alerting_metadata: metadata,
        user_info: Some(event),
        request_model: None,
        api_base: None,
        kwargs,
    }
}

#[tokio::test]
async fn a_single_slack_webhook_url_enqueues_one_item() {
    let fixture = slack_fixture();
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    let pending = fixture.delivery.queue.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].url.expose_secret(), "https://hooks/slack");
    assert_eq!(pending[0].alert_type, AlertType::LlmExceptions);
    assert_eq!(pending[0].format, None);
    assert_eq!(
        pending[0].headers.get("Content-type").map(String::as_str),
        Some("application/json")
    );
}

#[tokio::test]
async fn a_webhook_url_list_enqueues_one_item_per_url() {
    let mut fixture = fixture(&[], 512);
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
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    let pending = fixture.delivery.queue.pending();
    let urls: Vec<&str> = pending
        .iter()
        .map(|item| item.url.expose_secret())
        .collect();
    assert_eq!(urls, vec!["https://hooks/a", "https://hooks/b"]);
}

#[tokio::test]
async fn the_per_type_mapping_wins_over_the_default_and_the_environment() {
    let mut fixture = fixture(&[(SLACK_WEBHOOK_URL_ENV, "https://hooks/env")], 512);
    fixture.webhooks.insert(
        AlertType::LlmExceptions,
        WebhookUrl::Single(SecretString::from("https://hooks/type")),
    );
    fixture.default_webhook_url = Some(WebhookUrl::Single(SecretString::from("https://hooks/def")));
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    let pending = fixture.delivery.queue.pending();
    assert_eq!(pending[0].url.expose_secret(), "https://hooks/type");
}

#[tokio::test]
async fn the_default_webhook_url_wins_over_the_environment() {
    let mut fixture = fixture(&[(SLACK_WEBHOOK_URL_ENV, "https://hooks/env")], 512);
    fixture.default_webhook_url = Some(WebhookUrl::Single(SecretString::from("https://hooks/def")));
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    let pending = fixture.delivery.queue.pending();
    assert_eq!(pending[0].url.expose_secret(), "https://hooks/def");
}

async fn assert_single_slack_alert(environment: &[(&str, &str)], expected: &str) {
    let fixture = fixture(environment, 512);
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    assert!(
        dispatcher
            .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
            .await
            .is_ok()
    );
    let pending = fixture.delivery.queue.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].url.expose_secret(), expected);
}

#[tokio::test]
async fn a_slack_webhook_url_wins_over_the_alerting_url() {
    assert_single_slack_alert(
        &[
            (SLACK_WEBHOOK_URL_ENV, "https://hooks/slack"),
            (ALERTING_WEBHOOK_URL_ENV, "https://hooks/alerting"),
        ],
        "https://hooks/slack",
    )
    .await;
}

#[tokio::test]
async fn the_alerting_webhook_url_fills_in_when_slack_is_unset() {
    assert_single_slack_alert(
        &[(ALERTING_WEBHOOK_URL_ENV, "https://hooks/alerting")],
        "https://hooks/alerting",
    )
    .await;
}

#[tokio::test]
async fn an_empty_slack_webhook_url_falls_back_to_the_alerting_url() {
    assert_single_slack_alert(
        &[
            (SLACK_WEBHOOK_URL_ENV, ""),
            (ALERTING_WEBHOOK_URL_ENV, "https://hooks/alerting"),
        ],
        "https://hooks/alerting",
    )
    .await;
}

#[tokio::test]
async fn an_empty_alerting_webhook_url_raises() {
    let fixture = fixture(&[(ALERTING_WEBHOOK_URL_ENV, "")], 512);
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(matches!(sent, Err(SendAlertError::MissingWebhookUrl(_))));
    assert!(fixture.delivery.queue.pending().is_empty());
}

#[tokio::test]
async fn a_missing_webhook_url_raises_when_neither_variable_is_set() {
    let fixture = fixture(&[], 512);
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(matches!(sent, Err(SendAlertError::MissingWebhookUrl(_))));
    assert!(fixture.delivery.queue.pending().is_empty());
}

#[test]
fn destinations_serialize_to_the_configuration_names() {
    let names: Vec<String> = [
        AlertDestination::Slack,
        AlertDestination::Email,
        AlertDestination::MsTeams,
        AlertDestination::Sentry,
        AlertDestination::Webhook,
    ]
    .iter()
    .map(|destination| serde_json::to_string(destination).expect("a destination serializes"))
    .collect();
    assert_eq!(
        names,
        [
            "\"slack\"",
            "\"email\"",
            "\"ms_teams\"",
            "\"sentry\"",
            "\"webhook\""
        ]
    );
}

#[test]
fn webhook_environment_variables_use_the_litellm_names() {
    assert_eq!(SLACK_WEBHOOK_URL_ENV, "SLACK_WEBHOOK_URL");
    assert_eq!(ALERTING_WEBHOOK_URL_ENV, "ALERTING_WEBHOOK_URL");
}

#[test]
fn the_missing_webhook_url_error_names_both_variables() {
    assert_eq!(
        MissingWebhookUrl.to_string(),
        "Missing SLACK_WEBHOOK_URL / ALERTING_WEBHOOK_URL from environment"
    );
}

#[tokio::test]
async fn an_empty_per_type_webhook_url_list_is_used_as_given() {
    let mut fixture = slack_fixture();
    fixture
        .webhooks
        .insert(AlertType::LlmExceptions, WebhookUrl::Multiple(Vec::new()));
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    assert!(fixture.delivery.queue.pending().is_empty());
    assert_eq!(fixture.delivery.delivery.request_count(), 0);
}

#[tokio::test]
async fn an_unset_alerting_drops_the_alert_without_raising() {
    let fixture = slack_fixture();
    let dispatcher = fixture.dispatcher(None, vec![AlertType::LlmExceptions]);
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    assert!(fixture.delivery.queue.pending().is_empty());
    assert!(fixture.sink.calls().is_empty());
}

#[tokio::test]
async fn an_empty_alerting_drops_the_alert_without_raising() {
    let fixture = slack_fixture();
    let dispatcher = fixture.dispatcher(Some(Vec::new()), vec![AlertType::LlmExceptions]);
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    assert!(fixture.delivery.queue.pending().is_empty());
}

#[tokio::test]
async fn destinations_without_slack_or_teams_drop_the_alert() {
    let fixture = slack_fixture();
    let dispatcher = fixture.dispatcher(
        Some(vec![
            AlertDestination::Webhook,
            AlertDestination::Email,
            AlertDestination::Sentry,
        ]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    assert!(fixture.delivery.queue.pending().is_empty());
    assert!(fixture.sink.calls().is_empty());
}

#[tokio::test]
async fn an_unselected_alert_type_is_dropped_without_enqueuing() {
    let fixture = slack_fixture();
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmTooSlow],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    assert!(fixture.delivery.queue.pending().is_empty());
}

#[tokio::test]
async fn a_teams_alert_is_enqueued_with_the_teams_format() {
    let fixture = fixture(&[(MS_TEAMS_WEBHOOK_URL_ENV, "https://hooks/teams")], 512);
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::MsTeams]),
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
}

#[tokio::test]
async fn a_teams_alert_without_a_webhook_url_is_dropped_without_raising() {
    let fixture = fixture(&[], 512);
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::MsTeams]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    assert!(fixture.delivery.queue.pending().is_empty());
}

#[tokio::test]
async fn a_budget_alert_sends_to_the_webhook_when_only_webhook_is_configured() {
    let fixture = fixture(&[], 512);
    let event = budget_event();
    let metadata = IndexMap::new();
    let kwargs = IndexMap::new();
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Webhook]),
        vec![AlertType::BudgetAlerts],
    );
    let sent = dispatcher
        .send_alert(alert_with_event(
            AlertType::BudgetAlerts,
            &event,
            &metadata,
            &kwargs,
        ))
        .await;
    assert!(sent.is_ok());
    assert_eq!(
        fixture.sink.calls(),
        vec![SinkCall::Webhook("budget_crossed".to_owned())]
    );
    assert!(fixture.delivery.queue.pending().is_empty());
}

#[tokio::test]
async fn a_budget_alert_sends_to_the_webhook_before_email() {
    let fixture = fixture(&[], 512);
    let event = budget_event();
    let metadata = IndexMap::new();
    let kwargs = IndexMap::new();
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Webhook, AlertDestination::Email]),
        vec![AlertType::BudgetAlerts],
    );
    let sent = dispatcher
        .send_alert(alert_with_event(
            AlertType::BudgetAlerts,
            &event,
            &metadata,
            &kwargs,
        ))
        .await;
    assert!(sent.is_ok());
    assert_eq!(
        fixture.sink.calls(),
        vec![
            SinkCall::Webhook("budget_crossed".to_owned()),
            SinkCall::Email("budget_crossed".to_owned(), AlertType::BudgetAlerts),
        ]
    );
}

#[tokio::test]
async fn a_budget_alert_sends_to_the_webhook_even_when_its_type_is_not_selected() {
    let fixture = slack_fixture();
    let event = budget_event();
    let metadata = IndexMap::new();
    let kwargs = IndexMap::new();
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Webhook, AlertDestination::Slack]),
        Vec::new(),
    );
    let sent = dispatcher
        .send_alert(alert_with_event(
            AlertType::BudgetAlerts,
            &event,
            &metadata,
            &kwargs,
        ))
        .await;
    assert!(sent.is_ok());
    assert_eq!(
        fixture.sink.calls(),
        vec![SinkCall::Webhook("budget_crossed".to_owned())]
    );
    assert!(fixture.delivery.queue.pending().is_empty());
}

#[tokio::test]
async fn a_budget_alert_without_user_info_skips_the_webhook_and_email() {
    let fixture = fixture(&[], 512);
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Webhook, AlertDestination::Email]),
        vec![AlertType::BudgetAlerts],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("budget crossed", AlertType::BudgetAlerts))
        .await;
    assert!(sent.is_ok());
    assert!(fixture.sink.calls().is_empty());
}

#[tokio::test]
async fn a_non_budget_alert_never_reaches_the_budget_sink() {
    let fixture = slack_fixture();
    let event = budget_event();
    let metadata = IndexMap::new();
    let kwargs = IndexMap::new();
    let dispatcher = fixture.dispatcher(
        Some(vec![
            AlertDestination::Webhook,
            AlertDestination::Email,
            AlertDestination::Slack,
        ]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(Alert {
            message: "boom",
            level: AlertLevel::High,
            alert_type: AlertType::LlmExceptions,
            alerting_metadata: &metadata,
            user_info: Some(&event),
            request_model: None,
            api_base: None,
            kwargs: &kwargs,
        })
        .await;
    assert!(sent.is_ok());
    assert!(fixture.sink.calls().is_empty());
    assert_eq!(fixture.delivery.queue.pending().len(), 1);
}

#[tokio::test]
async fn a_budget_alert_still_enqueues_a_slack_alert_when_slack_is_configured() {
    let fixture = slack_fixture();
    let event = budget_event();
    let metadata = IndexMap::new();
    let kwargs = IndexMap::new();
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::BudgetAlerts],
    );
    let sent = dispatcher
        .send_alert(alert_with_event(
            AlertType::BudgetAlerts,
            &event,
            &metadata,
            &kwargs,
        ))
        .await;
    assert!(sent.is_ok());
    assert_eq!(fixture.delivery.queue.pending().len(), 1);
}

#[tokio::test]
async fn a_webhook_failure_stops_the_email_and_slack_dispatch() {
    let mut fixture = slack_fixture();
    fixture.sink.fail_webhook = true;
    let event = budget_event();
    let metadata = IndexMap::new();
    let kwargs = IndexMap::new();
    let dispatcher = fixture.dispatcher(
        Some(vec![
            AlertDestination::Webhook,
            AlertDestination::Email,
            AlertDestination::Slack,
        ]),
        vec![AlertType::BudgetAlerts],
    );
    let sent = dispatcher
        .send_alert(alert_with_event(
            AlertType::BudgetAlerts,
            &event,
            &metadata,
            &kwargs,
        ))
        .await;
    assert!(matches!(sent, Err(SendAlertError::Budget(_))));
    assert_eq!(
        fixture.sink.calls(),
        vec![SinkCall::Webhook("budget_crossed".to_owned())]
    );
    assert!(fixture.delivery.queue.pending().is_empty());
}

#[tokio::test]
async fn an_email_failure_is_returned_after_the_webhook_call() {
    let mut fixture = slack_fixture();
    fixture.sink.fail_email = true;
    let event = budget_event();
    let metadata = IndexMap::new();
    let kwargs = IndexMap::new();
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Webhook, AlertDestination::Email]),
        vec![AlertType::BudgetAlerts],
    );
    let sent = dispatcher
        .send_alert(alert_with_event(
            AlertType::BudgetAlerts,
            &event,
            &metadata,
            &kwargs,
        ))
        .await;
    assert!(matches!(sent, Err(SendAlertError::Budget(_))));
    assert_eq!(
        fixture.sink.calls(),
        vec![
            SinkCall::Webhook("budget_crossed".to_owned()),
            SinkCall::Email("budget_crossed".to_owned(), AlertType::BudgetAlerts),
        ]
    );
    assert!(fixture.delivery.queue.pending().is_empty());
}
