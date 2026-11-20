#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::MetricValue;
use chrono::DateTime;

use super::gauge::EntityBudgetGauges;
use super::inputs::KeyBudgetGaugeInput;
use super::key::set_key_budget_metrics;
use super::test_support::{decimal, events, field, finite, hours_value, new_log, set, source};
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus_label_schema::PrometheusLabelSettings;

const REMAINING: &str = "litellm_remaining_api_key_budget_metric";
const MAX: &str = "litellm_api_key_max_budget_metric";
const HOURS: &str = "litellm_api_key_budget_remaining_hours_metric";

fn gauges_with_distinct_lists() -> (
    EntityBudgetGauges<super::test_support::ProbeGauge>,
    super::test_support::ProbeLog,
) {
    let log = new_log();
    let gauges = EntityBudgetGauges {
        remaining: field(REMAINING, &["hashed_api_key"], &log),
        max: field(MAX, &["api_key_alias"], &log),
        remaining_hours: field(HOURS, &["team"], &log),
    };
    (gauges, log)
}

fn key(max: Option<&str>, reset: Option<DateTime<chrono::Utc>>) -> KeyBudgetGaugeInput {
    KeyBudgetGaugeInput {
        token: "sk-1".to_owned(),
        key_alias: Some("paid".to_owned()),
        spend: Some(decimal("4")),
        max_budget: max.map(decimal),
        budget_reset_at: reset,
    }
}

#[test]
fn the_hours_gauge_reuses_the_max_budget_labels() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = gauges_with_distinct_lists();
    let input = key(Some("10"), Some(DateTime::from_timestamp(7200, 0).unwrap()));
    set_key_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(
        events(&log),
        vec![
            set(REMAINING, &[("hashed_api_key", "sk-1")], finite("6")),
            set(MAX, &[("api_key_alias", "paid")], finite("10")),
            set(HOURS, &[("api_key_alias", "paid")], hours_value(2.0)),
        ]
    );
}

#[test]
fn the_hours_gauge_reuses_the_remaining_labels_without_a_max_budget() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = gauges_with_distinct_lists();
    let input = key(None, Some(DateTime::from_timestamp(7200, 0).unwrap()));
    set_key_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(
        events(&log),
        vec![
            set(
                REMAINING,
                &[("hashed_api_key", "sk-1")],
                MetricValue::PositiveInfinity,
            ),
            set(HOURS, &[("hashed_api_key", "sk-1")], hours_value(2.0)),
        ]
    );
}

#[test]
fn no_reset_time_writes_no_hours_gauge() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = gauges_with_distinct_lists();
    let input = key(Some("10"), None);
    set_key_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(
        events(&log),
        vec![
            set(REMAINING, &[("hashed_api_key", "sk-1")], finite("6")),
            set(MAX, &[("api_key_alias", "paid")], finite("10")),
        ]
    );
}
