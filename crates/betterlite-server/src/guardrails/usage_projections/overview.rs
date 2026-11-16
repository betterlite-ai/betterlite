use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use serde::Deserialize;

use super::EMPTY_UNITS;
use super::aggregation::first_match;
use super::fields::{DEFAULT_GUARDRAIL_TYPE, guardrail_attrs, guardrail_provider, guardrail_type};
use super::metrics::{MetricTotals, metric_rate};
use super::records::{GuardrailRecord, PolicyRecord};
use super::response::UsageOverviewRow;
use super::status::{status_from_fail_rate, trend_from_comparison};

/// What the overview rows read: per-key metrics, the previous window's rates and the unit maps.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct UsageAggregates {
    pub metrics: BTreeMap<String, MetricTotals>,
    pub previous_rates: BTreeMap<String, f64>,
    pub units: BTreeMap<String, BTreeMap<String, i64>>,
    pub costs: BTreeMap<String, Option<Decimal>>,
    pub untracked: BTreeMap<String, BTreeMap<String, i64>>,
}

/// One row per guardrail record, then one per metrics key no record covers.
#[must_use]
pub fn guardrail_overview_rows(
    guardrails: &[GuardrailRecord],
    aggregates: &UsageAggregates,
) -> Vec<UsageOverviewRow> {
    let mut rows = Vec::new();
    let mut covered = BTreeSet::new();
    for record in guardrails {
        rows.push(guardrail_row(record, aggregates, &mut covered));
    }
    rows.extend(metric_only_rows(&covered, aggregates));
    rows
}

fn guardrail_row(
    record: &GuardrailRecord,
    aggregates: &UsageAggregates,
    covered: &mut BTreeSet<String>,
) -> UsageOverviewRow {
    let (id, name) = guardrail_attrs(record);
    // metrics are keyed by the logical name from spend log metadata, so both keys are tried in order
    let lookup_keys: Vec<&str> = [name.as_str(), id.as_str()]
        .into_iter()
        .filter(|key| !key.is_empty())
        .collect();
    covered.extend(lookup_keys.iter().map(|key| (*key).to_owned()));
    let totals = *first_match(&lookup_keys, &aggregates.metrics, &MetricTotals::default());
    let rate = metric_rate(totals.blocked, totals.requests);
    let previous = *first_match(&lookup_keys, &aggregates.previous_rates, &0.0);
    let units = first_match(&lookup_keys, &aggregates.units, &EMPTY_UNITS).clone();
    let untracked = first_match(&lookup_keys, &aggregates.untracked, &EMPTY_UNITS).clone();
    let cost = *first_match(&lookup_keys, &aggregates.costs, &None);
    UsageOverviewRow {
        id,
        name,
        kind: guardrail_type(record),
        provider: guardrail_provider(record),
        requests_evaluated: totals.requests,
        fail_rate: rate.rounded,
        avg_score: None,
        avg_latency: None,
        status: status_from_fail_rate(rate.exact),
        trend: trend_from_comparison(rate.exact, previous),
        usage_units: units,
        cost,
        untracked_usage_units: untracked,
    }
}

fn metric_only_rows(
    covered: &BTreeSet<String>,
    aggregates: &UsageAggregates,
) -> Vec<UsageOverviewRow> {
    let mut rows = Vec::new();
    for (key, totals) in &aggregates.metrics {
        if covered.contains(key) || totals.requests == 0 {
            continue;
        }
        let rate = metric_rate(totals.blocked, totals.requests);
        let previous = aggregates.previous_rates.get(key).copied().unwrap_or(0.0);
        rows.push(UsageOverviewRow {
            id: key.clone(),
            name: key.clone(),
            kind: DEFAULT_GUARDRAIL_TYPE.to_owned(),
            provider: "Custom".to_owned(),
            requests_evaluated: totals.requests,
            fail_rate: rate.rounded,
            avg_score: None,
            avg_latency: None,
            status: status_from_fail_rate(rate.exact),
            trend: trend_from_comparison(rate.exact, previous),
            usage_units: aggregates.units.get(key).cloned().unwrap_or_default(),
            cost: aggregates.costs.get(key).copied().flatten(),
            untracked_usage_units: aggregates.untracked.get(key).cloned().unwrap_or_default(),
        });
    }
    rows
}

/// One row per policy, looked up by the policy's own id.
#[must_use]
pub fn policy_overview_rows(
    policies: &[PolicyRecord],
    aggregates: &UsageAggregates,
) -> Vec<UsageOverviewRow> {
    let mut rows = Vec::new();
    for policy in policies {
        let id = policy.policy_id.clone();
        let totals = aggregates.metrics.get(&id).copied().unwrap_or_default();
        let rate = metric_rate(totals.blocked, totals.requests);
        let previous = aggregates.previous_rates.get(&id).copied().unwrap_or(0.0);
        let name = policy
            .policy_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&id)
            .to_owned();
        rows.push(UsageOverviewRow {
            id,
            name,
            kind: "Policy".to_owned(),
            provider: "LiteLLM".to_owned(),
            requests_evaluated: totals.requests,
            fail_rate: rate.rounded,
            avg_score: None,
            avg_latency: None,
            status: status_from_fail_rate(rate.exact),
            trend: trend_from_comparison(rate.exact, previous),
            usage_units: BTreeMap::new(),
            cost: None,
            untracked_usage_units: BTreeMap::new(),
        });
    }
    rows
}

mod tests;
