#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::MetricValue;
use chrono::DateTime;

use super::set_team_budget_metrics;
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::budget_gauges::gauge::EntityBudgetGauges;
use crate::prometheus::budget_gauges::inputs::TeamBudgetGaugeInput;
use crate::prometheus::budget_gauges::test_support::{
    decimal, events, field, finite, hours_value, new_log, rig, set, source,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const REMAINING: &str = "litellm_remaining_team_budget_metric";
const MAX: &str = "litellm_team_max_budget_metric";
const HOURS: &str = "litellm_team_budget_remaining_hours_metric";

fn team(alias: Option<&str>, spend: Option<&str>, max: Option<&str>) -> TeamBudgetGaugeInput {
    TeamBudgetGaugeInput {
        team_id: "t1".to_owned(),
        team_alias: alias.map(str::to_owned),
        spend: spend.map(decimal),
        max_budget: max.map(decimal),
        budget_reset_at: Some(DateTime::from_timestamp(7200, 0).unwrap()),
    }
}

#[test]
fn all_three_gauges_are_written() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["team", "team_alias"]);
    let input = team(Some("Alpha"), Some("4"), Some("10"));
    set_team_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    let labels = [("team", "t1"), ("team_alias", "Alpha")];
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
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["team", "team_alias"]);
    let mut input = team(Some("Alpha"), Some("4"), None);
    input.budget_reset_at = None;
    set_team_budget_metrics(
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
            &[("team", "t1"), ("team_alias", "Alpha")],
            MetricValue::PositiveInfinity,
        )]
    );
}

#[test]
fn a_zero_max_budget_is_written_with_the_overspend_below_zero() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["team"]);
    let mut input = team(Some("Alpha"), Some("2"), Some("0"));
    input.budget_reset_at = None;
    input.team_alias = None;
    set_team_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    let labels = [("team", "t1")];
    assert_eq!(
        events(&log),
        vec![
            set(REMAINING, &labels, finite("-2")),
            set(MAX, &labels, finite("0")),
        ]
    );
}

#[test]
fn an_absent_alias_is_written_as_an_empty_label() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(REMAINING, MAX, HOURS, &["team", "team_alias"]);
    let mut input = team(None, Some("1"), None);
    input.budget_reset_at = None;
    set_team_budget_metrics(
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
            &[("team", "t1"), ("team_alias", "")],
            MetricValue::PositiveInfinity,
        )]
    );
}

#[test]
fn each_gauge_builds_its_labels_from_its_own_list() {
    let settings = PrometheusLabelSettings::with_defaults();
    let log = new_log();
    let gauges = EntityBudgetGauges {
        remaining: field(REMAINING, &["team"], &log),
        max: field(MAX, &["team_alias"], &log),
        remaining_hours: field(HOURS, &["team_alias"], &log),
    };
    let input = team(Some("Alpha"), Some("4"), Some("10"));
    set_team_budget_metrics(
        &gauges,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(
        events(&log),
        vec![
            set(REMAINING, &[("team", "t1")], finite("6")),
            set(MAX, &[("team_alias", "Alpha")], finite("10")),
            set(HOURS, &[("team_alias", "Alpha")], hours_value(2.0)),
        ]
    );
}
