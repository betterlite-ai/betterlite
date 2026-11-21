#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::MetricValue;
use chrono::DateTime;

use super::set_key_budget_metrics;
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::budget_gauges::inputs::KeyBudgetGaugeInput;
use crate::prometheus::budget_gauges::test_support::{
    decimal, events, finite, hours_value, rig, set, source,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const REMAINING: &str = "litellm_remaining_api_key_budget_metric";
const MAX: &str = "litellm_api_key_max_budget_metric";
const HOURS: &str = "litellm_api_key_budget_remaining_hours_metric";

fn key(alias: Option<&str>, spend: Option<&str>, max: Option<&str>) -> KeyBudgetGaugeInput {
    KeyBudgetGaugeInput {
        token: "sk-1".to_owned(),
        key_alias: alias.map(str::to_owned),
        spend: spend.map(decimal),
        max_budget: max.map(decimal),
        budget_reset_at: Some(DateTime::from_timestamp(7200, 0).unwrap()),
    }
}

#[test]
fn all_three_gauges_are_written() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["hashed_api_key", "api_key_alias"]);
    let input = key(Some("paid"), Some("4"), Some("10"));
    set_key_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    let labels = [("api_key_alias", "paid"), ("hashed_api_key", "sk-1")];
    assert_eq!(
        events(&log),
        vec![
            set(REMAINING, &labels, finite("6")),
            set(MAX, &labels, finite("10")),
            set(HOURS, &labels, hours_value(2.0)),
        ]
    );
}

#[test]
fn no_limits_write_only_the_remaining_gauge() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["hashed_api_key", "api_key_alias"]);
    let mut input = key(None, Some("4"), None);
    input.budget_reset_at = None;
    set_key_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(
        events(&log),
        vec![set(
            REMAINING,
            &[("api_key_alias", ""), ("hashed_api_key", "sk-1")],
            MetricValue::PositiveInfinity,
        )]
    );
}

#[test]
fn a_zero_max_budget_is_still_written() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["hashed_api_key"]);
    let mut input = key(Some("paid"), Some("2"), Some("0"));
    input.budget_reset_at = None;
    set_key_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    let labels = [("hashed_api_key", "sk-1")];
    assert_eq!(
        events(&log),
        vec![
            set(REMAINING, &labels, finite("-2")),
            set(MAX, &labels, finite("0")),
        ]
    );
}
