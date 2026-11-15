#![cfg(test)]

use betterlite_logging::alert_dispatch::destination::AlertDestination;
use betterlite_logging::budget_email::event::{EntityType, WebhookEventType};
use secrecy::ExposeSecret as _;

use super::BudgetAlertError;
use super::CustomerSpend;
use super::support::{FakeCache, FakeSender, decimal, service};

fn customer_spend() -> CustomerSpend<'static> {
    CustomerSpend {
        token: Some("hashed-key"),
        key_alias: Some("key-1"),
        end_user_id: Some("cust-1"),
        response_cost: Some(decimal("0.5")),
        max_budget: Some(decimal("10")),
    }
}

#[tokio::test]
async fn no_destinations_configured_sends_nothing() {
    let sender = FakeSender::default();
    let mut alerts = service(FakeCache::default(), sender.clone());
    alerts.alerting = None;

    let result = alerts.customer_spend_alert(customer_spend()).await;

    result.unwrap();
    assert!(sender.webhooks.entries().is_empty());
}

#[tokio::test]
async fn a_non_webhook_destination_sends_nothing() {
    let sender = FakeSender::default();
    let mut alerts = service(FakeCache::default(), sender.clone());
    alerts.alerting = Some(vec![AlertDestination::Slack, AlertDestination::Email]);

    let result = alerts.customer_spend_alert(customer_spend()).await;

    result.unwrap();
    assert!(sender.webhooks.entries().is_empty());
}

#[tokio::test]
async fn a_missing_end_user_sends_nothing() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut spend = customer_spend();
    spend.end_user_id = None;

    let result = alerts.customer_spend_alert(spend).await;

    result.unwrap();
    assert!(sender.webhooks.entries().is_empty());
}

#[tokio::test]
async fn a_missing_key_sends_nothing() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut spend = customer_spend();
    spend.token = None;

    let result = alerts.customer_spend_alert(spend).await;

    result.unwrap();
    assert!(sender.webhooks.entries().is_empty());
}

#[tokio::test]
async fn a_missing_cost_sends_nothing() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut spend = customer_spend();
    spend.response_cost = None;

    let result = alerts.customer_spend_alert(spend).await;

    result.unwrap();
    assert!(sender.webhooks.entries().is_empty());
}

#[tokio::test]
async fn the_spend_event_names_the_customer_and_the_cost() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());

    let result = alerts.customer_spend_alert(customer_spend()).await;

    result.unwrap();
    let webhooks = sender.webhooks.entries();
    let event = webhooks.first().expect("one spend event");
    assert_eq!(event.event, WebhookEventType::SpendTracked);
    assert_eq!(event.event_group, EntityType::EndUser);
    assert_eq!(event.customer_id.as_deref(), Some("cust-1"));
    assert_eq!(event.spend, decimal("0.5"));
    assert_eq!(event.max_budget, Some(decimal("10")));
    assert_eq!(event.key_alias.as_deref(), Some("key-1"));
    let token = event.token.as_ref().expect("the event carries the key");
    assert_eq!(token.expose_secret(), "hashed-key");
    assert_eq!(
        event.event_message,
        "Customer spend tracked. Customer=cust-1, spend=0.5"
    );
    assert!(event.soft_budget.is_none());
    assert!(event.user_id.is_none());
    assert!(event.team_id.is_none());
    assert!(event.user_email.is_none());
    assert!(event.projected_spend.is_none());
}

#[tokio::test]
async fn a_missing_max_budget_is_allowed() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut spend = customer_spend();
    spend.max_budget = None;

    let result = alerts.customer_spend_alert(spend).await;

    result.unwrap();
    let webhooks = sender.webhooks.entries();
    let event = webhooks.first().expect("one spend event");
    assert!(event.max_budget.is_none());
}

#[tokio::test]
async fn a_failed_webhook_send_is_reported() {
    let alerts = service(FakeCache::default(), FakeSender::failing());

    let result = alerts.customer_spend_alert(customer_spend()).await;

    assert!(matches!(result, Err(BudgetAlertError::Webhook(_))));
}
