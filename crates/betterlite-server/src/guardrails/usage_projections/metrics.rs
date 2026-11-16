use std::collections::BTreeMap;
use std::ops::{Div, Mul};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::records::UsageMetricRow;
use super::response::UsageChartPoint;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricTotals {
    pub requests: i64,
    pub passed: i64,
    pub blocked: i64,
    pub flagged: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FailRate {
    pub(crate) exact: f64,
    pub(crate) rounded: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct PeriodCounts {
    requests: i64,
    blocked: i64,
}

#[derive(Debug, Clone, Copy, Default)]
struct DailyCounts {
    passed: i64,
    blocked: i64,
}

// Decimal's as_f64 is the infallible i64-to-f64 conversion, so no `as` cast sneaks in
fn fail_rate(blocked: i64, requests: i64) -> f64 {
    let blocked_share = Decimal::from(blocked).as_f64();
    let request_total = Decimal::from(requests).as_f64();
    Div::div(Mul::mul(100.0_f64, blocked_share), request_total)
}

// Python's round(rate, 1): the nearest float to the one-decimal rounding of the exact binary value
fn round_one_decimal(rate: f64) -> f64 {
    let Some(exact) = Decimal::from_f64_retain(rate) else {
        return rate;
    };
    exact.round_dp(1).as_f64()
}

pub(crate) fn metric_rate(blocked: i64, requests: i64) -> FailRate {
    if requests == 0 {
        return FailRate {
            exact: 0.0,
            rounded: 0.0,
        };
    }
    let exact = fail_rate(blocked, requests);
    FailRate {
        exact,
        rounded: round_one_decimal(exact),
    }
}

#[must_use]
pub fn aggregate_daily_metrics(metrics: &[UsageMetricRow]) -> BTreeMap<String, MetricTotals> {
    let mut totals: BTreeMap<String, MetricTotals> = BTreeMap::new();
    for row in metrics {
        let entry = totals.entry(row.id.clone()).or_default();
        entry.requests = entry
            .requests
            .saturating_add(row.requests_evaluated.unwrap_or(0));
        entry.passed = entry.passed.saturating_add(row.passed_count.unwrap_or(0));
        entry.blocked = entry.blocked.saturating_add(row.blocked_count.unwrap_or(0));
        entry.flagged = entry.flagged.saturating_add(row.flagged_count.unwrap_or(0));
    }
    totals
}

// an id with no requests counts as a zero fail rate
#[must_use]
pub fn prev_fail_rates(metrics: &[UsageMetricRow]) -> BTreeMap<String, f64> {
    let mut counts: BTreeMap<String, PeriodCounts> = BTreeMap::new();
    for row in metrics {
        let entry = counts.entry(row.id.clone()).or_default();
        entry.requests = entry
            .requests
            .saturating_add(row.requests_evaluated.unwrap_or(0));
        entry.blocked = entry.blocked.saturating_add(row.blocked_count.unwrap_or(0));
    }
    counts
        .into_iter()
        .map(|(id, period)| {
            if period.requests == 0 {
                (id, 0.0_f64)
            } else {
                (id, fail_rate(period.blocked, period.requests))
            }
        })
        .collect()
}

#[must_use]
pub fn chart_from_metrics(metrics: &[UsageMetricRow]) -> Vec<UsageChartPoint> {
    let mut days: BTreeMap<String, DailyCounts> = BTreeMap::new();
    for row in metrics {
        let entry = days.entry(row.date.clone()).or_default();
        entry.passed = entry.passed.saturating_add(row.passed_count.unwrap_or(0));
        entry.blocked = entry.blocked.saturating_add(row.blocked_count.unwrap_or(0));
    }
    days.into_iter()
        .map(|(date, daily)| UsageChartPoint {
            date,
            passed: daily.passed,
            blocked: daily.blocked,
            score: None,
        })
        .collect()
}

mod tests;
