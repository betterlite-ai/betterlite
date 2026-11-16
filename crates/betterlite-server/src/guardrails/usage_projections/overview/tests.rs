#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use rust_decimal::Decimal;

use super::{UsageAggregates, guardrail_overview_rows, policy_overview_rows};
use crate::guardrails::usage_projections::metrics::MetricTotals;
use crate::guardrails::usage_projections::records::{
    GuardrailRecord, PolicyRecord, UsageGuardrailParams,
};
use crate::guardrails::usage_projections::status::{UsageStatus, UsageTrend};

fn empty_aggregates() -> UsageAggregates {
    UsageAggregates::default()
}

fn totals(requests: i64, passed: i64, blocked: i64) -> MetricTotals {
    MetricTotals {
        requests,
        passed,
        blocked,
        flagged: 0,
    }
}

fn record(id: &str, name: Option<&str>, provider: &str) -> GuardrailRecord {
    GuardrailRecord {
        guardrail_id: Some(id.to_owned()),
        guardrail_name: name.map(str::to_owned),
        litellm_params: UsageGuardrailParams {
            guardrail: Some(provider.to_owned()),
        },
        guardrail_info: None,
    }
}

fn policy(id: &str, name: Option<&str>) -> PolicyRecord {
    PolicyRecord {
        policy_id: id.to_owned(),
        policy_name: name.map(str::to_owned),
    }
}

#[test]
fn guardrail_overview_looks_metrics_up_by_display_name_first() {
    let lakera = record("uuid-1", Some("lakera"), "lakera");
    let mut aggregates = empty_aggregates();
    aggregates
        .metrics
        .insert("lakera".to_owned(), totals(10, 8, 2));
    aggregates
        .metrics
        .insert("uuid-1".to_owned(), totals(50, 50, 0));
    let rows = guardrail_overview_rows(&[lakera], &aggregates);
    assert_eq!(rows.len(), 1);
    let row = rows.first().unwrap();
    assert_eq!(row.id, "uuid-1");
    assert_eq!(row.name, "lakera");
    assert_eq!(row.requests_evaluated, 10);
    assert_eq!(row.fail_rate.to_bits(), 20.0_f64.to_bits());
    assert_eq!(row.status, UsageStatus::Critical);
}

#[test]
fn guardrail_overview_falls_back_to_the_id_key() {
    let lakera = record("uuid-1", Some("lakera"), "lakera");
    let mut aggregates = empty_aggregates();
    aggregates
        .metrics
        .insert("uuid-1".to_owned(), totals(4, 3, 1));
    let rows = guardrail_overview_rows(&[lakera], &aggregates);
    let row = rows.first().unwrap();
    assert_eq!(row.requests_evaluated, 4);
    assert_eq!(row.fail_rate.to_bits(), 25.0_f64.to_bits());
}

#[test]
fn guardrail_overview_defaults_absent_metrics_to_zero() {
    let lakera = record("uuid-1", Some("lakera"), "lakera");
    let rows = guardrail_overview_rows(&[lakera], &empty_aggregates());
    let row = rows.first().unwrap();
    assert_eq!(row.requests_evaluated, 0);
    assert_eq!(row.fail_rate.to_bits(), 0.0_f64.to_bits());
    assert_eq!(row.status, UsageStatus::Healthy);
    assert_eq!(row.trend, UsageTrend::Stable);
    assert!(row.usage_units.is_empty());
    assert_eq!(row.cost, None);
    assert!(row.untracked_usage_units.is_empty());
}

#[test]
fn guardrail_overview_reads_provider_type_units_and_cost() {
    let mut lakera = record("uuid-1", Some("lakera"), "presidio");
    lakera.guardrail_info = Some(OpaqueParams(BTreeMap::from([(
        "type".to_owned(),
        GuardrailValue::Text("pii".to_owned()),
    )])));
    let mut aggregates = empty_aggregates();
    aggregates
        .metrics
        .insert("lakera".to_owned(), totals(10, 8, 2));
    aggregates.units.insert(
        "lakera".to_owned(),
        BTreeMap::from([("requests".to_owned(), 10)]),
    );
    aggregates.untracked.insert(
        "lakera".to_owned(),
        BTreeMap::from([("requests".to_owned(), 3)]),
    );
    aggregates
        .costs
        .insert("lakera".to_owned(), Some(Decimal::new(175, 2)));
    let rows = guardrail_overview_rows(&[lakera], &aggregates);
    let row = rows.first().unwrap();
    assert_eq!(row.provider, "presidio");
    assert_eq!(row.kind, "pii");
    assert_eq!(row.usage_units.get("requests"), Some(&10));
    assert_eq!(row.untracked_usage_units.get("requests"), Some(&3));
    assert_eq!(row.cost, Some(Decimal::new(175, 2)));
}

#[test]
fn guardrail_overview_keeps_an_unpriced_cost_null() {
    let lakera = record("uuid-1", Some("lakera"), "lakera");
    let mut aggregates = empty_aggregates();
    aggregates
        .metrics
        .insert("lakera".to_owned(), totals(2, 2, 0));
    aggregates.costs.insert("lakera".to_owned(), None);
    let rows = guardrail_overview_rows(&[lakera], &aggregates);
    assert_eq!(rows.first().unwrap().cost, None);
}

#[test]
fn guardrail_overview_adds_a_custom_row_for_an_unlisted_metric_key() {
    let mut aggregates = empty_aggregates();
    aggregates
        .metrics
        .insert("config-guard".to_owned(), totals(4, 3, 1));
    aggregates.units.insert(
        "config-guard".to_owned(),
        BTreeMap::from([("requests".to_owned(), 4)]),
    );
    let rows = guardrail_overview_rows(&[], &aggregates);
    assert_eq!(rows.len(), 1);
    let row = rows.first().unwrap();
    assert_eq!(row.id, "config-guard");
    assert_eq!(row.name, "config-guard");
    assert_eq!(row.kind, "Guardrail");
    assert_eq!(row.provider, "Custom");
    assert_eq!(row.requests_evaluated, 4);
    assert_eq!(row.fail_rate.to_bits(), 25.0_f64.to_bits());
    assert_eq!(row.usage_units.get("requests"), Some(&4));
    assert_eq!(row.cost, None);
}

#[test]
fn guardrail_overview_omits_unlisted_metric_keys_with_no_requests() {
    let mut aggregates = empty_aggregates();
    aggregates
        .metrics
        .insert("unused".to_owned(), totals(0, 0, 0));
    assert!(guardrail_overview_rows(&[], &aggregates).is_empty());
}

#[test]
fn guardrail_overview_reads_the_previous_rate_for_the_trend() {
    let lakera = record("uuid-1", Some("lakera"), "lakera");
    let mut aggregates = empty_aggregates();
    aggregates
        .metrics
        .insert("lakera".to_owned(), totals(10, 8, 2));
    aggregates.previous_rates.insert("lakera".to_owned(), 10.0);
    aggregates.previous_rates.insert("uuid-1".to_owned(), 90.0);
    let rows = guardrail_overview_rows(&[lakera], &aggregates);
    assert_eq!(rows.first().unwrap().trend, UsageTrend::Up);
}

#[test]
fn guardrail_overview_uses_the_previous_rate_of_a_custom_row() {
    let mut aggregates = empty_aggregates();
    aggregates
        .metrics
        .insert("mcp-guard".to_owned(), totals(6, 2, 4));
    aggregates
        .previous_rates
        .insert("mcp-guard".to_owned(), 10.0);
    let rows = guardrail_overview_rows(&[], &aggregates);
    assert_eq!(rows.first().unwrap().trend, UsageTrend::Up);
    assert_eq!(rows.first().unwrap().status, UsageStatus::Critical);
}

#[test]
fn policy_overview_looks_rows_up_by_policy_id() {
    let pii = policy("pol-1", Some("PII policy"));
    let mut aggregates = empty_aggregates();
    aggregates
        .metrics
        .insert("pol-1".to_owned(), totals(20, 18, 2));
    aggregates.previous_rates.insert("pol-1".to_owned(), 10.0);
    let rows = policy_overview_rows(&[pii], &aggregates);
    assert_eq!(rows.len(), 1);
    let row = rows.first().unwrap();
    assert_eq!(row.id, "pol-1");
    assert_eq!(row.name, "PII policy");
    assert_eq!(row.kind, "Policy");
    assert_eq!(row.provider, "LiteLLM");
    assert_eq!(row.requests_evaluated, 20);
    assert_eq!(row.fail_rate.to_bits(), 10.0_f64.to_bits());
    assert_eq!(row.status, UsageStatus::Warning);
    assert_eq!(row.trend, UsageTrend::Stable);
}

#[test]
fn policy_overview_defaults_absent_metrics_and_hides_units() {
    let pii = policy("pol-1", Some("PII policy"));
    let mut aggregates = empty_aggregates();
    aggregates.units.insert(
        "pol-1".to_owned(),
        BTreeMap::from([("requests".to_owned(), 9)]),
    );
    aggregates
        .costs
        .insert("pol-1".to_owned(), Some(Decimal::new(125, 2)));
    let rows = policy_overview_rows(&[pii], &aggregates);
    let row = rows.first().unwrap();
    assert_eq!(row.requests_evaluated, 0);
    assert_eq!(row.fail_rate.to_bits(), 0.0_f64.to_bits());
    assert!(row.usage_units.is_empty());
    assert!(row.untracked_usage_units.is_empty());
    assert_eq!(row.cost, None);
}

#[test]
fn policy_overview_names_an_unnamed_policy_by_its_id() {
    let unnamed = policy("pol-1", None);
    let empty = policy("pol-2", Some(""));
    let rows = policy_overview_rows(&[unnamed, empty], &empty_aggregates());
    assert_eq!(rows.first().unwrap().name, "pol-1");
    assert_eq!(rows.get(1).unwrap().name, "pol-2");
}

#[test]
fn policy_overview_rounds_the_fail_rate() {
    let pii = policy("pol-1", Some("PII policy"));
    let mut aggregates = empty_aggregates();
    aggregates
        .metrics
        .insert("pol-1".to_owned(), totals(3, 2, 1));
    let rows = policy_overview_rows(&[pii], &aggregates);
    assert_eq!(
        rows.first().unwrap().fail_rate.to_bits(),
        33.3_f64.to_bits()
    );
}
