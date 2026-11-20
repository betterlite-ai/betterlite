#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::MetricValue;
use chrono::DateTime;

use super::set_org_budget_metrics;
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::budget_gauges::inputs::OrgBudgetGaugeInput;
use crate::prometheus::budget_gauges::test_support::{
    decimal, events, finite, hours_value, rig, set, source,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const REMAINING: &str = "litellm_remaining_org_budget_metric";
const MAX: &str = "litellm_org_max_budget_metric";
const HOURS: &str = "litellm_org_budget_remaining_hours_metric";

fn org(alias: &str, spend: &str, max: Option<&str>) -> OrgBudgetGaugeInput {
    OrgBudgetGaugeInput {
        org_id: "o1".to_owned(),
        org_alias: alias.to_owned(),
        spend: decimal(spend),
        max_budget: max.map(decimal),
        budget_reset_at: Some(DateTime::from_timestamp(7200, 0).unwrap()),
    }
}

#[test]
fn all_three_gauges_are_written() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["org_id", "org_alias"]);
    let input = org("Org", "4", Some("10"));
    set_org_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    let labels = [("org_alias", "Org"), ("org_id", "o1")];
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
fn no_budget_limits_write_only_the_remaining_gauge() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["org_id", "org_alias"]);
    let mut input = org("", "4", None);
    input.budget_reset_at = None;
    set_org_budget_metrics(
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
            &[("org_alias", ""), ("org_id", "o1")],
            MetricValue::PositiveInfinity,
        )]
    );
}

#[test]
fn a_zero_spend_keeps_the_whole_limit() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["org_id"]);
    let mut input = org("Org", "0", Some("10"));
    input.budget_reset_at = None;
    set_org_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    let labels = [("org_id", "o1")];
    assert_eq!(
        events(&log),
        vec![
            set(REMAINING, &labels, finite("10")),
            set(MAX, &labels, finite("10")),
        ]
    );
}

#[test]
fn a_past_reset_time_writes_a_negative_hours_value() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["org_id"]);
    let mut input = org("Org", "1", None);
    input.budget_reset_at = Some(DateTime::from_timestamp(0, 0).unwrap());
    set_org_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(3600, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(
        events(&log),
        vec![
            set(
                REMAINING,
                &[("org_id", "o1")],
                MetricValue::PositiveInfinity
            ),
            set(HOURS, &[("org_id", "o1")], hours_value(-1.0)),
        ]
    );
}
