use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::capture_rate::format::{serialize_money, serialize_optional_money};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::benchmark_aggregates::row::{SessionAggRow, compared_row};
use crate::benchmark_aggregates::totals::{BenchmarkTotals, CacheStats, benchmark_totals};
use crate::benchmark_aggregates::window::BenchmarkWindow;
use crate::routing::benchmark_aggregates::{StrategyDeployment, idle_router_groups};

// AutoRouterBenchmarkGroup flattened: a serde flatten next to arbitrary precision numbers breaks them
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkGroup {
    pub router_name: String,
    pub router_type: String,
    pub tier_turns: BTreeMap<String, i64>,
    pub sessions: i64,
    pub turns: i64,
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub avg_turns_per_session: Decimal,
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub avg_session_seconds: Decimal,
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub avg_tokens_per_session: Decimal,
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub spend: Decimal,
    #[serde(
        serialize_with = "serialize_optional_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision_option::deserialize"
    )]
    pub classifier_cost: Option<Decimal>,
    pub savings_estimated_turns: i64,
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub savings_estimated_actual_spend: Decimal,
    #[serde(
        rename = "savings_estimated_classifier_cost",
        serialize_with = "serialize_optional_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision_option::deserialize"
    )]
    pub savings_classifier_cost: Option<Decimal>,
    #[serde(
        serialize_with = "serialize_optional_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision_option::deserialize"
    )]
    pub saved_spend: Option<Decimal>,
    #[serde(
        serialize_with = "serialize_optional_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision_option::deserialize"
    )]
    pub baseline_spend: Option<Decimal>,
    #[serde(
        serialize_with = "serialize_optional_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision_option::deserialize"
    )]
    pub saved_pct: Option<Decimal>,
    #[serde(
        serialize_with = "serialize_optional_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision_option::deserialize"
    )]
    pub saved_per_session: Option<Decimal>,
    pub cache: CacheStats,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Benchmarks {
    pub start_date: String,
    pub end_date: String,
    pub routers_in_scope: usize,
    pub totals: BenchmarkTotals,
    pub groups: Vec<BenchmarkGroup>,
}

#[must_use]
pub fn benchmark_group(row: &SessionAggRow) -> BenchmarkGroup {
    let totals = benchmark_totals(row);
    BenchmarkGroup {
        router_name: row.router_name.clone(),
        router_type: row.router_type.clone(),
        tier_turns: row.tier_turns.clone(),
        sessions: totals.sessions,
        turns: totals.turns,
        avg_turns_per_session: totals.avg_turns_per_session,
        avg_session_seconds: totals.avg_session_seconds,
        avg_tokens_per_session: totals.avg_tokens_per_session,
        spend: totals.spend,
        classifier_cost: totals.classifier_cost,
        savings_estimated_turns: totals.savings_estimated_turns,
        savings_estimated_actual_spend: totals.savings_estimated_actual_spend,
        savings_classifier_cost: totals.savings_classifier_cost,
        saved_spend: totals.saved_spend,
        baseline_spend: totals.baseline_spend,
        saved_pct: totals.saved_pct,
        saved_per_session: totals.saved_per_session,
        cache: totals.cache,
    }
}

fn summed_counts<F>(rows: &[SessionAggRow], field: F) -> i64
where
    F: Fn(&SessionAggRow) -> i64,
{
    let mut total = 0_i64;
    for row in rows {
        total = total.saturating_add(field(row));
    }
    total
}

fn summed_money<F>(rows: &[SessionAggRow], field: F) -> Decimal
where
    F: Fn(&SessionAggRow) -> Decimal,
{
    let mut total = Decimal::ZERO;
    for row in rows {
        total = total.saturating_add(field(row));
    }
    total
}

fn summed_optional_money<F>(rows: &[SessionAggRow], field: F) -> Option<Decimal>
where
    F: Fn(&SessionAggRow) -> Option<Decimal>,
{
    let mut total = Some(Decimal::ZERO);
    for row in rows {
        total = total
            .zip(field(row))
            .map(|(summed, amount)| summed.saturating_add(amount));
    }
    total
}

#[must_use]
pub fn summed_agg_row(rows: &[SessionAggRow]) -> SessionAggRow {
    SessionAggRow {
        router_name: String::new(),
        router_type: String::new(),
        tier_turns: BTreeMap::new(),
        sessions: summed_counts(rows, |row| row.sessions),
        turns: summed_counts(rows, |row| row.turns),
        unordered_turns: summed_counts(rows, |row| row.unordered_turns),
        covered_turns: summed_counts(rows, |row| row.covered_turns),
        cache_hits: summed_counts(rows, |row| row.cache_hits),
        same_model_turns: summed_counts(rows, |row| row.same_model_turns),
        same_model_hits: summed_counts(rows, |row| row.same_model_hits),
        first_visit_turns: summed_counts(rows, |row| row.first_visit_turns),
        first_visit_hits: summed_counts(rows, |row| row.first_visit_hits),
        return_turns: summed_counts(rows, |row| row.return_turns),
        return_hits: summed_counts(rows, |row| row.return_hits),
        return_expired_misses: summed_counts(rows, |row| row.return_expired_misses),
        return_within_ttl_misses: summed_counts(rows, |row| row.return_within_ttl_misses),
        ttl_5m_turns: summed_counts(rows, |row| row.ttl_5m_turns),
        ttl_1h_turns: summed_counts(rows, |row| row.ttl_1h_turns),
        total_tokens: summed_counts(rows, |row| row.total_tokens),
        spend: summed_money(rows, |row| row.spend),
        saved_spend: summed_money(rows, |row| row.saved_spend),
        savings_estimated_turns: summed_counts(rows, |row| row.savings_estimated_turns),
        savings_estimated_actual_spend: summed_money(rows, |row| {
            row.savings_estimated_actual_spend
        }),
        savings_classifier_cost: summed_optional_money(rows, |row| row.savings_classifier_cost),
        savings_estimated_saved_spend: summed_money(rows, |row| row.savings_estimated_saved_spend),
        classifier_cost: summed_money(rows, |row| row.classifier_cost),
        classifier_cost_recorded_turns: summed_counts(rows, |row| {
            row.classifier_cost_recorded_turns
        }),
        session_seconds: summed_money(rows, |row| row.session_seconds),
    }
}

#[must_use]
pub fn benchmarks(
    window: &BenchmarkWindow,
    rows: &[SessionAggRow],
    deployments: Option<&[StrategyDeployment<'_>]>,
) -> Benchmarks {
    let compared: Vec<SessionAggRow> = rows.iter().map(compared_row).collect();
    let covered: BTreeSet<(String, String)> = compared
        .iter()
        .map(|row| (row.router_name.clone(), row.router_type.clone()))
        .collect();
    let mut groups: Vec<BenchmarkGroup> = compared.iter().map(benchmark_group).collect();
    groups.extend(idle_router_groups(deployments, &covered));
    Benchmarks {
        start_date: window.start_date.clone(),
        end_date: window.end_date.clone(),
        routers_in_scope: groups.len(),
        totals: benchmark_totals(&summed_agg_row(&compared)),
        groups,
    }
}
