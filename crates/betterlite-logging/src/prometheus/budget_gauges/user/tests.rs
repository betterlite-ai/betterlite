#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::MetricValue;
use chrono::DateTime;

use super::set_user_budget_metrics;
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::budget_gauges::inputs::UserBudgetGaugeInput;
use crate::prometheus::budget_gauges::test_support::{
    decimal, events, finite, hours_value, rig, set, source,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const REMAINING: &str = "litellm_remaining_user_budget_metric";
const MAX: &str = "litellm_user_max_budget_metric";
const HOURS: &str = "litellm_user_budget_remaining_hours_metric";

fn user(
    email: Option<&str>,
    alias: Option<&str>,
    spend: Option<&str>,
    max: Option<&str>,
) -> UserBudgetGaugeInput {
    UserBudgetGaugeInput {
        user_id: "u1".to_owned(),
        user_email: email.map(str::to_owned),
        user_alias: alias.map(str::to_owned),
        spend: spend.map(decimal),
        max_budget: max.map(decimal),
        budget_reset_at: Some(DateTime::from_timestamp(7200, 0).unwrap()),
    }
}

#[test]
fn all_three_gauges_are_written() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["user", "user_email", "user_alias"]);
    let input = user(Some("a@b.c"), Some("Ali"), Some("4"), Some("10"));
    set_user_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    let labels = [
        ("user", "u1"),
        ("user_alias", "Ali"),
        ("user_email", "a@b.c"),
    ];
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
fn an_absent_email_and_alias_are_written_as_empty_labels() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["user", "user_email", "user_alias"]);
    let mut input = user(None, None, Some("1"), None);
    input.budget_reset_at = None;
    set_user_budget_metrics(
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
            &[("user", "u1"), ("user_alias", ""), ("user_email", "")],
            MetricValue::PositiveInfinity,
        )]
    );
}

#[test]
fn a_max_budget_alone_writes_the_remaining_and_max_gauges() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["user"]);
    let mut input = user(None, None, Some("3"), Some("10"));
    input.budget_reset_at = None;
    set_user_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    let labels = [("user", "u1")];
    assert_eq!(
        events(&log),
        vec![
            set(REMAINING, &labels, finite("7")),
            set(MAX, &labels, finite("10")),
        ]
    );
}
