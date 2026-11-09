use rust_decimal::Decimal;

use super::support::{attribute_at, envelope, metric_names, record};
use crate::newrelic_metrics::wire::{NewRelicMetric, NewRelicMetricEnvelope};

const WINDOW_START_MS: u64 = 1_700_000_000_000;
const WINDOW_END_MS: u64 = 1_700_000_001_500;

fn gauges(envelope: &NewRelicMetricEnvelope) -> Vec<Decimal> {
    envelope
        .metrics
        .iter()
        .filter_map(|metric| match metric {
            NewRelicMetric::Gauge { value, .. } => Some(*value),
            NewRelicMetric::Count { .. } | NewRelicMetric::Summary { .. } => None,
        })
        .collect()
}

#[test]
fn gauges_come_from_the_latest_record_of_each_team() {
    let mut first = record();
    first.team_max_budget = Some(Decimal::new(200, 1));
    first.team_spend = Some(Decimal::new(55, 1));
    first.response_cost = Decimal::new(15, 1);
    let mut second = record();
    second.model = "gpt-4o-mini".to_owned();
    second.team_max_budget = Some(Decimal::new(100, 1));
    second.team_spend = Some(Decimal::new(25, 1));
    second.response_cost = Decimal::new(5, 1);
    let envelope = envelope(&[first, second], WINDOW_START_MS, WINDOW_END_MS);
    assert_eq!(metric_names(&envelope).len(), 14);
    assert_eq!(
        gauges(&envelope),
        [Decimal::new(100, 1), Decimal::new(70, 1)]
    );
}

#[test]
fn gauges_pair_max_and_remaining_for_each_team() {
    let mut first = record();
    first.team_id = "team-b".to_owned();
    first.team_max_budget = Some(Decimal::new(200, 1));
    first.team_spend = Some(Decimal::new(55, 1));
    first.response_cost = Decimal::new(15, 1);
    let mut second = record();
    second.team_id = "team-a".to_owned();
    second.model = "gpt-4o-mini".to_owned();
    second.team_max_budget = Some(Decimal::new(100, 1));
    second.team_spend = Some(Decimal::new(25, 1));
    second.response_cost = Decimal::new(5, 1);
    let envelope = envelope(&[first, second], WINDOW_START_MS, WINDOW_END_MS);
    assert_eq!(
        gauges(&envelope),
        [
            Decimal::new(200, 1),
            Decimal::new(130, 1),
            Decimal::new(100, 1),
            Decimal::new(70, 1),
        ]
    );
    assert_eq!(attribute_at(&envelope, 12, "team_id"), "team-b");
    assert_eq!(attribute_at(&envelope, 14, "team_id"), "team-a");
}

#[test]
fn missing_team_max_budget_emits_no_gauges() {
    let mut record = record();
    record.team_max_budget = None;
    let envelope = envelope(&[record], WINDOW_START_MS, WINDOW_END_MS);
    assert!(gauges(&envelope).is_empty());
    assert_eq!(metric_names(&envelope).len(), 6);
}

#[test]
fn records_with_an_empty_team_id_emit_no_gauges() {
    let mut record = record();
    record.team_id = String::new();
    let envelope = envelope(&[record], WINDOW_START_MS, WINDOW_END_MS);
    assert!(gauges(&envelope).is_empty());
}

#[test]
fn remaining_budget_is_not_clamped_at_zero() {
    let mut record = record();
    record.team_max_budget = Some(Decimal::new(100, 1));
    record.team_spend = Some(Decimal::new(120, 1));
    record.response_cost = Decimal::new(5, 1);
    let envelope = envelope(&[record], WINDOW_START_MS, WINDOW_END_MS);
    assert_eq!(
        gauges(&envelope),
        [Decimal::new(100, 1), Decimal::new(-25, 1)]
    );
}

#[test]
fn a_latest_record_without_a_budget_drops_the_teams_gauges() {
    let mut first = record();
    first.team_max_budget = Some(Decimal::new(100, 1));
    let mut second = record();
    second.team_max_budget = None;
    let envelope = envelope(&[first, second], WINDOW_START_MS, WINDOW_END_MS);
    assert!(gauges(&envelope).is_empty());
}

#[test]
fn a_zero_max_budget_still_emits_gauges() {
    let mut record = record();
    record.team_max_budget = Some(Decimal::ZERO);
    record.team_spend = Some(Decimal::new(25, 1));
    record.response_cost = Decimal::new(5, 1);
    let envelope = envelope(&[record], WINDOW_START_MS, WINDOW_END_MS);
    assert_eq!(gauges(&envelope), [Decimal::ZERO, Decimal::new(-30, 1)]);
}

#[test]
fn remaining_budget_treats_a_missing_team_spend_as_zero() {
    let mut record = record();
    record.team_max_budget = Some(Decimal::new(100, 1));
    record.team_spend = None;
    record.response_cost = Decimal::new(5, 1);
    let envelope = envelope(&[record], WINDOW_START_MS, WINDOW_END_MS);
    assert_eq!(
        gauges(&envelope),
        [Decimal::new(100, 1), Decimal::new(95, 1)]
    );
}
