#![cfg(test)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::{EMPTY_OVERVIEW, UsageChartPoint, UsageOverviewResponse, UsageOverviewRow};
use crate::guardrails::usage_projections::status::{UsageStatus, UsageTrend};

fn row() -> UsageOverviewRow {
    UsageOverviewRow {
        id: "uuid-1".to_owned(),
        name: "lakera".to_owned(),
        kind: "pii".to_owned(),
        provider: "presidio".to_owned(),
        requests_evaluated: 10,
        fail_rate: 20.0,
        avg_score: None,
        avg_latency: None,
        status: UsageStatus::Critical,
        trend: UsageTrend::Up,
        usage_units: BTreeMap::from([("requests".to_owned(), 10)]),
        cost: Some(Decimal::new(175, 2)),
        untracked_usage_units: BTreeMap::from([("requests".to_owned(), 3)]),
    }
}

#[test]
fn empty_overview_serializes_the_litellm_default_payload() {
    let serialized = serde_json::to_string(&EMPTY_OVERVIEW).unwrap();
    assert_eq!(
        serialized,
        r#"{"rows":[],"chart":[],"totalRequests":0,"totalBlocked":0,"passRate":100.0,"totalUsageUnits":{},"totalCost":null,"totalUntrackedUsageUnits":{}}"#
    );
}

#[test]
fn overview_row_serializes_the_camel_case_keys() {
    let serialized = serde_json::to_string(&row()).unwrap();
    assert_eq!(
        serialized,
        r#"{"id":"uuid-1","name":"lakera","type":"pii","provider":"presidio","requestsEvaluated":10,"failRate":20.0,"avgScore":null,"avgLatency":null,"status":"critical","trend":"up","usageUnits":{"requests":10},"cost":1.75,"untrackedUsageUnits":{"requests":3}}"#
    );
}

#[test]
fn overview_row_reads_the_wire_payload_back() {
    let source = r#"{"id":"uuid-1","name":"lakera","type":"pii","provider":"presidio","requestsEvaluated":10,"failRate":20.0,"avgScore":0.5,"avgLatency":12.0,"status":"critical","trend":"up","usageUnits":{"requests":10},"cost":1.75,"untrackedUsageUnits":{"requests":3}}"#;
    let parsed: UsageOverviewRow = serde_json::from_str(source).unwrap();
    assert_eq!(parsed.id, "uuid-1");
    assert_eq!(parsed.kind, "pii");
    assert_eq!(parsed.provider, "presidio");
    assert_eq!(parsed.requests_evaluated, 10);
    assert_eq!(parsed.fail_rate.to_bits(), 20.0_f64.to_bits());
    assert_eq!(parsed.avg_score.map(f64::to_bits), Some(0.5_f64.to_bits()));
    assert_eq!(
        parsed.avg_latency.map(f64::to_bits),
        Some(12.0_f64.to_bits())
    );
    assert_eq!(parsed.status, UsageStatus::Critical);
    assert_eq!(parsed.trend, UsageTrend::Up);
    assert_eq!(parsed.usage_units.get("requests"), Some(&10));
    assert_eq!(parsed.cost, Some(Decimal::new(175, 2)));
    assert_eq!(parsed.untracked_usage_units.get("requests"), Some(&3));
}

#[test]
fn overview_row_reads_a_null_cost_and_an_absent_score() {
    let source = r#"{"id":"uuid-1","name":"lakera","type":"Guardrail","provider":"Custom","requestsEvaluated":0,"failRate":0.0,"avgScore":null,"avgLatency":null,"status":"healthy","trend":"stable","usageUnits":{},"cost":null,"untrackedUsageUnits":{}}"#;
    let parsed: UsageOverviewRow = serde_json::from_str(source).unwrap();
    assert_eq!(parsed.cost, None);
    assert_eq!(parsed.avg_score, None);
    assert_eq!(parsed.status, UsageStatus::Healthy);
    assert_eq!(parsed.trend, UsageTrend::Stable);
}

#[test]
fn overview_response_reads_a_priced_total_cost() {
    let source = r#"{"rows":[],"chart":[],"totalRequests":0,"totalBlocked":0,"passRate":100.0,"totalUsageUnits":{"requests":4},"totalCost":2.5,"totalUntrackedUsageUnits":{}}"#;
    let parsed: UsageOverviewResponse = serde_json::from_str(source).unwrap();
    assert_eq!(parsed.total_cost, Some(Decimal::new(25, 1)));
    assert_eq!(parsed.total_usage_units.get("requests"), Some(&4));
    assert_eq!(parsed.pass_rate.to_bits(), 100.0_f64.to_bits());
}

#[test]
fn chart_point_leaves_an_absent_score_out_of_the_payload() {
    let point = UsageChartPoint {
        date: "2026-11-01".to_owned(),
        passed: 8,
        blocked: 2,
        score: None,
    };
    assert_eq!(
        serde_json::to_string(&point).unwrap(),
        r#"{"date":"2026-11-01","passed":8,"blocked":2}"#
    );
    let parsed: UsageChartPoint =
        serde_json::from_str(r#"{"date":"2026-11-01","passed":8,"blocked":2}"#).unwrap();
    assert_eq!(parsed.score, None);
}

#[test]
fn chart_point_keeps_a_present_score() {
    let point = UsageChartPoint {
        date: "2026-11-01".to_owned(),
        passed: 8,
        blocked: 2,
        score: Some(0.5),
    };
    assert_eq!(
        serde_json::to_string(&point).unwrap(),
        r#"{"date":"2026-11-01","passed":8,"blocked":2,"score":0.5}"#
    );
    let parsed: UsageChartPoint =
        serde_json::from_str(r#"{"date":"2026-11-01","passed":8,"blocked":2,"score":0.5}"#)
            .unwrap();
    assert_eq!(parsed.score.map(f64::to_bits), Some(0.5_f64.to_bits()));
}
