#![cfg(test)]

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use crate::datadog_llm_privacy::record_object;
use crate::datadog_llm_privacy::spend::spend_metrics;
use crate::datadog_llm_privacy::tests::support;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::payload::records::DecimalValue;

fn metrics_object(payload: &StandardLoggingPayload) -> Json {
    let object =
        record_object("spend_metrics", &spend_metrics(payload)).expect("the metrics encode");
    Json::Object(object)
}

#[test]
fn spend_metrics_carry_the_cost_budgets_and_the_reset_in_utc() {
    let mut payload = support::payload();
    payload.metadata.user_api_key_max_budget = Some(DecimalValue(Decimal::new(225, 2)));
    payload.metadata.user_api_key_spend = Some(DecimalValue(Decimal::new(5, 1)));
    payload.metadata.user_api_key_budget_reset_at = Some("2026-01-01T00:00:00".to_owned());
    let metrics = spend_metrics(&payload);
    assert_eq!(metrics.response_cost, Some(Decimal::new(5, 1)));
    assert_eq!(metrics.user_api_key_max_budget, Some(Decimal::new(225, 2)));
    assert_eq!(metrics.user_api_key_spend, Some(Decimal::new(5, 1)));
    assert_eq!(
        metrics.user_api_key_budget_reset_at.as_deref(),
        Some("2026-01-01T00:00:00+00:00")
    );
}

#[test]
fn spend_metrics_omit_a_budget_the_metadata_does_not_carry() {
    let payload = support::payload();
    let metrics = spend_metrics(&payload);
    assert_eq!(metrics.response_cost, Some(Decimal::new(5, 1)));
    assert_eq!(metrics.user_api_key_max_budget, None);
    assert_eq!(metrics.user_api_key_spend, None);
    assert_eq!(metrics.user_api_key_budget_reset_at, None);
}

#[test]
fn spend_metrics_drop_an_invalid_budget_reset() {
    let mut payload = support::payload();
    payload.metadata.user_api_key_budget_reset_at = Some("next tuesday".to_owned());
    assert_eq!(spend_metrics(&payload).user_api_key_budget_reset_at, None);
}

#[test]
fn spend_metrics_object_writes_every_present_field() {
    let mut payload = support::payload();
    payload.metadata.user_api_key_max_budget = Some(DecimalValue(Decimal::new(225, 2)));
    payload.metadata.user_api_key_spend = Some(DecimalValue(Decimal::new(5, 1)));
    payload.metadata.user_api_key_budget_reset_at = Some("2026-01-01T00:00:00Z".to_owned());
    let metrics = metrics_object(&payload);
    let Json::Object(object) = metrics else {
        panic!("the metrics are an object");
    };
    assert_eq!(object.get("response_cost"), Some(&support::json("0.5")));
    assert_eq!(
        object.get("user_api_key_spend"),
        Some(&support::json("0.5"))
    );
    assert_eq!(
        object.get("user_api_key_max_budget"),
        Some(&support::json("2.25"))
    );
    assert_eq!(
        object.get("user_api_key_budget_reset_at"),
        Some(&support::json("\"2026-01-01T00:00:00+00:00\""))
    );
}
