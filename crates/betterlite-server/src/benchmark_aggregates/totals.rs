use std::ops::{Div, Mul};

use betterlite_models::capture_rate::format::{serialize_money, serialize_optional_money};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::alerting::spend_reports::float_rounded;
use crate::benchmark_aggregates::row::{SessionAggRow, savings_cohort};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheBucket {
    pub turns: i64,
    pub hits: i64,
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub hit_rate_pct: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheStats {
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub coverage_pct: Decimal,
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub hit_rate_pct: Decimal,
    pub same_model: CacheBucket,
    pub first_visit: CacheBucket,
    pub return_to_tier: CacheBucket,
    pub unordered_turns: i64,
    pub return_misses_expired: i64,
    pub return_misses_within_ttl: i64,
    pub return_misses_unknown: i64,
    pub ttl_5m_turns: i64,
    pub ttl_1h_turns: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkTotals {
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

// LiteLLM's _pct: a non-positive denominator is 0.0, and Python's float round keeps one decimal
#[must_use]
pub fn pct(numerator: Decimal, denominator: Decimal) -> Decimal {
    if denominator <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let share = Div::div(
        Mul::mul(100.0_f64, numerator.as_f64()),
        denominator.as_f64(),
    );
    float_rounded(Decimal::from_f64_retain(share).unwrap_or_default(), 1)
}

fn average(total: Decimal, sessions: i64) -> Decimal {
    if sessions == 0 {
        return Decimal::ZERO;
    }
    total
        .checked_div(Decimal::from(sessions))
        .unwrap_or_default()
}

#[must_use]
pub fn benchmark_totals(row: &SessionAggRow) -> BenchmarkTotals {
    let return_misses = row.return_turns.saturating_sub(row.return_hits);
    let (saved_spend, baseline_spend) = savings_cohort(
        row.turns,
        row.savings_estimated_turns,
        row.savings_estimated_actual_spend,
        row.savings_estimated_saved_spend,
    );
    let sessions = row.sessions;
    BenchmarkTotals {
        sessions,
        turns: row.turns,
        avg_turns_per_session: average(Decimal::from(row.turns), sessions),
        avg_session_seconds: average(row.session_seconds, sessions),
        avg_tokens_per_session: average(Decimal::from(row.total_tokens), sessions),
        spend: row.spend,
        classifier_cost: (row.classifier_cost_recorded_turns == row.turns)
            .then_some(row.classifier_cost),
        savings_estimated_turns: row.savings_estimated_turns,
        savings_estimated_actual_spend: row.savings_estimated_actual_spend,
        savings_estimated_classifier_cost: row.savings_estimated_classifier_cost,
        saved_spend,
        baseline_spend,
        saved_pct: saved_spend
            .zip(baseline_spend)
            .map(|(saved, baseline)| pct(saved, baseline)),
        saved_per_session: saved_spend.map(|saved| average(saved, sessions)),
        cache: CacheStats {
            coverage_pct: pct(Decimal::from(row.covered_turns), Decimal::from(row.turns)),
            hit_rate_pct: pct(
                Decimal::from(row.cache_hits),
                Decimal::from(row.covered_turns),
            ),
            same_model: CacheBucket {
                turns: row.same_model_turns,
                hits: row.same_model_hits,
                hit_rate_pct: pct(
                    Decimal::from(row.same_model_hits),
                    Decimal::from(row.same_model_turns),
                ),
            },
            first_visit: CacheBucket {
                turns: row.first_visit_turns,
                hits: row.first_visit_hits,
                hit_rate_pct: pct(
                    Decimal::from(row.first_visit_hits),
                    Decimal::from(row.first_visit_turns),
                ),
            },
            return_to_tier: CacheBucket {
                turns: row.return_turns,
                hits: row.return_hits,
                hit_rate_pct: pct(
                    Decimal::from(row.return_hits),
                    Decimal::from(row.return_turns),
                ),
            },
            unordered_turns: row.unordered_turns,
            return_misses_expired: row.return_expired_misses,
            return_misses_within_ttl: row.return_within_ttl_misses,
            return_misses_unknown: return_misses
                .saturating_sub(row.return_expired_misses)
                .saturating_sub(row.return_within_ttl_misses)
                .max(0),
            ttl_5m_turns: row.ttl_5m_turns,
            ttl_1h_turns: row.ttl_1h_turns,
        },
    }
}
