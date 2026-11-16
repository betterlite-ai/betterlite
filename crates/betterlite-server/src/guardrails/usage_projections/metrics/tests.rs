#![cfg(test)]

use std::ops::Div;

use super::{
    MetricTotals, aggregate_daily_metrics, chart_from_metrics, metric_rate, prev_fail_rates,
};
use crate::guardrails::usage_projections::records::UsageMetricRow;

fn row(date: &str, id: &str, requests: i64, passed: i64, blocked: i64) -> UsageMetricRow {
    UsageMetricRow {
        date: date.to_owned(),
        id: id.to_owned(),
        requests_evaluated: Some(requests),
        passed_count: Some(passed),
        blocked_count: Some(blocked),
        flagged_count: Some(0),
    }
}

fn flagged(mut row: UsageMetricRow, count: i64) -> UsageMetricRow {
    row.flagged_count = Some(count);
    row
}

#[test]
fn metrics_aggregation_sums_all_four_counters_per_id() {
    let metrics = [
        flagged(row("2026-11-01", "lakera", 10, 8, 1), 2),
        row("2026-11-02", "lakera", 5, 4, 1),
        flagged(row("2026-11-02", "presidio", 3, 1, 2), 1),
    ];
    let totals = aggregate_daily_metrics(&metrics);
    assert_eq!(
        totals.get("lakera"),
        Some(&MetricTotals {
            requests: 15,
            passed: 12,
            blocked: 2,
            flagged: 2
        })
    );
    assert_eq!(
        totals.get("presidio"),
        Some(&MetricTotals {
            requests: 3,
            passed: 1,
            blocked: 2,
            flagged: 1
        })
    );
}

#[test]
fn metrics_aggregation_treats_null_counters_as_zero() {
    let metrics = [
        UsageMetricRow {
            date: "2026-11-01".to_owned(),
            id: "lakera".to_owned(),
            requests_evaluated: None,
            passed_count: Some(4),
            blocked_count: None,
            flagged_count: None,
        },
        row("2026-11-02", "lakera", 2, 1, 1),
    ];
    let totals = aggregate_daily_metrics(&metrics);
    assert_eq!(
        totals.get("lakera"),
        Some(&MetricTotals {
            requests: 2,
            passed: 5,
            blocked: 1,
            flagged: 0
        })
    );
}

#[test]
fn metrics_aggregation_over_no_rows_is_empty() {
    assert!(aggregate_daily_metrics(&[]).is_empty());
}

#[test]
fn metrics_aggregation_rounds_each_rate_like_python() {
    assert_eq!(metric_rate(1, 2000).rounded.to_bits(), 0.1_f64.to_bits());
    assert_eq!(metric_rate(1, 400).rounded.to_bits(), 0.2_f64.to_bits());
    assert_eq!(metric_rate(3, 400).rounded.to_bits(), 0.8_f64.to_bits());
    assert_eq!(metric_rate(23, 2000).rounded.to_bits(), 1.1_f64.to_bits());
    assert_eq!(metric_rate(1, 3).rounded.to_bits(), 33.3_f64.to_bits());
    assert_eq!(
        metric_rate(1, 3).exact.to_bits(),
        100.0_f64.div(3.0).to_bits()
    );
    assert_eq!(metric_rate(16, 100).rounded.to_bits(), 16.0_f64.to_bits());
    assert_eq!(metric_rate(0, 3).rounded.to_bits(), 0.0_f64.to_bits());
}

#[test]
fn metrics_aggregation_gives_a_zero_rate_when_nothing_was_evaluated() {
    let rate = metric_rate(4, 0);
    assert_eq!(rate.exact.to_bits(), 0.0_f64.to_bits());
    assert_eq!(rate.rounded.to_bits(), 0.0_f64.to_bits());
}

#[test]
fn metrics_aggregation_prev_rates_divide_blocked_by_requests() {
    let previous = [
        row("2026-10-30", "lakera", 10, 8, 1),
        row("2026-10-31", "lakera", 30, 27, 3),
        row("2026-10-31", "presidio", 4, 3, 1),
        row("2026-10-31", "unused", 0, 0, 0),
    ];
    let rates = prev_fail_rates(&previous);
    assert_eq!(
        rates.get("lakera").copied().map(f64::to_bits),
        Some(10.0_f64.to_bits())
    );
    assert_eq!(
        rates.get("presidio").copied().map(f64::to_bits),
        Some(25.0_f64.to_bits())
    );
    assert_eq!(
        rates.get("unused").copied().map(f64::to_bits),
        Some(0.0_f64.to_bits())
    );
    assert_eq!(rates.len(), 3);
}

#[test]
fn metrics_aggregation_chart_groups_days_in_chronological_order() {
    let metrics = [
        row("2026-11-02", "lakera", 4, 3, 1),
        row("2026-11-01", "lakera", 10, 8, 2),
        row("2026-11-02", "presidio", 2, 1, 1),
    ];
    let chart = chart_from_metrics(&metrics);
    let dates: Vec<&str> = chart.iter().map(|point| point.date.as_str()).collect();
    assert_eq!(dates, ["2026-11-01", "2026-11-02"]);
    assert_eq!(chart.first().unwrap().passed, 8);
    assert_eq!(chart.first().unwrap().blocked, 2);
    assert_eq!(chart.get(1).unwrap().passed, 4);
    assert_eq!(chart.get(1).unwrap().blocked, 2);
}

#[test]
fn metrics_aggregation_chart_leaves_the_score_absent() {
    let chart = chart_from_metrics(&[row("2026-11-01", "lakera", 1, 1, 0)]);
    assert_eq!(chart.first().unwrap().score, None);
    let serialized = serde_json::to_string(&chart).unwrap();
    assert_eq!(
        serialized,
        r#"[{"date":"2026-11-01","passed":1,"blocked":0}]"#
    );
}
