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
        serialize_with = "serialize_optional_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision_option::deserialize"
    )]
    pub savings_estimated_classifier_cost: Option<Decimal>,
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
        savings_estimated_classifier_cost: totals.savings_estimated_classifier_cost,
        saved_spend: totals.saved_spend,
        baseline_spend: totals.baseline_spend,
        saved_pct: totals.saved_pct,
        saved_per_session: totals.saved_per_session,
        cache: totals.cache,
    }
}

#[must_use]
pub fn summed_agg_row(rows: &[SessionAggRow]) -> SessionAggRow {
    let savings_classifier_cost = rows
        .iter()
        .map(|row| row.savings_estimated_classifier_cost)
        .try_fold(Decimal::ZERO, |total, cost| {
            cost.map(|amount| total.saturating_add(amount))
        });
    SessionAggRow {
        router_name: String::new(),
        router_type: String::new(),
        tier_turns: BTreeMap::new(),
        sessions: rows
            .iter()
            .map(|row| row.sessions)
            .fold(0_i64, i64::saturating_add),
        turns: rows
            .iter()
            .map(|row| row.turns)
            .fold(0_i64, i64::saturating_add),
        unordered_turns: rows
            .iter()
            .map(|row| row.unordered_turns)
            .fold(0_i64, i64::saturating_add),
        covered_turns: rows
            .iter()
            .map(|row| row.covered_turns)
            .fold(0_i64, i64::saturating_add),
        cache_hits: rows
            .iter()
            .map(|row| row.cache_hits)
            .fold(0_i64, i64::saturating_add),
        same_model_turns: rows
            .iter()
            .map(|row| row.same_model_turns)
            .fold(0_i64, i64::saturating_add),
        same_model_hits: rows
            .iter()
            .map(|row| row.same_model_hits)
            .fold(0_i64, i64::saturating_add),
        first_visit_turns: rows
            .iter()
            .map(|row| row.first_visit_turns)
            .fold(0_i64, i64::saturating_add),
        first_visit_hits: rows
            .iter()
            .map(|row| row.first_visit_hits)
            .fold(0_i64, i64::saturating_add),
        return_turns: rows
            .iter()
            .map(|row| row.return_turns)
            .fold(0_i64, i64::saturating_add),
        return_hits: rows
            .iter()
            .map(|row| row.return_hits)
            .fold(0_i64, i64::saturating_add),
        return_expired_misses: rows
            .iter()
            .map(|row| row.return_expired_misses)
            .fold(0_i64, i64::saturating_add),
        return_within_ttl_misses: rows
            .iter()
            .map(|row| row.return_within_ttl_misses)
            .fold(0_i64, i64::saturating_add),
        ttl_5m_turns: rows
            .iter()
            .map(|row| row.ttl_5m_turns)
            .fold(0_i64, i64::saturating_add),
        ttl_1h_turns: rows
            .iter()
            .map(|row| row.ttl_1h_turns)
            .fold(0_i64, i64::saturating_add),
        total_tokens: rows
            .iter()
            .map(|row| row.total_tokens)
            .fold(0_i64, i64::saturating_add),
        spend: rows
            .iter()
            .map(|row| row.spend)
            .fold(Decimal::ZERO, Decimal::saturating_add),
        saved_spend: rows
            .iter()
            .map(|row| row.saved_spend)
            .fold(Decimal::ZERO, Decimal::saturating_add),
        savings_estimated_turns: rows
            .iter()
            .map(|row| row.savings_estimated_turns)
            .fold(0_i64, i64::saturating_add),
        savings_estimated_actual_spend: rows
            .iter()
            .map(|row| row.savings_estimated_actual_spend)
            .fold(Decimal::ZERO, Decimal::saturating_add),
        savings_estimated_classifier_cost: savings_classifier_cost,
        savings_estimated_saved_spend: rows
            .iter()
            .map(|row| row.savings_estimated_saved_spend)
            .fold(Decimal::ZERO, Decimal::saturating_add),
        classifier_cost: rows
            .iter()
            .map(|row| row.classifier_cost)
            .fold(Decimal::ZERO, Decimal::saturating_add),
        classifier_cost_recorded_turns: rows
            .iter()
            .map(|row| row.classifier_cost_recorded_turns)
            .fold(0_i64, i64::saturating_add),
        session_seconds: rows
            .iter()
            .map(|row| row.session_seconds)
            .fold(Decimal::ZERO, Decimal::saturating_add),
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
