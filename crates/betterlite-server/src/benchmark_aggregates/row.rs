use std::collections::BTreeMap;

use betterlite_models::capture_rate::format::{serialize_money, serialize_optional_money};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// LiteLLM's _SessionAggRow: one rollup row per router and type, all measures as recorded
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAggRow {
    pub router_name: String,
    pub router_type: String,
    pub tier_turns: BTreeMap<String, i64>,
    pub sessions: i64,
    pub turns: i64,
    pub unordered_turns: i64,
    pub covered_turns: i64,
    pub cache_hits: i64,
    pub same_model_turns: i64,
    pub same_model_hits: i64,
    pub first_visit_turns: i64,
    pub first_visit_hits: i64,
    pub return_turns: i64,
    pub return_hits: i64,
    pub return_expired_misses: i64,
    pub return_within_ttl_misses: i64,
    pub ttl_5m_turns: i64,
    pub ttl_1h_turns: i64,
    pub total_tokens: i64,
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub spend: Decimal,
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub saved_spend: Decimal,
    #[serde(default)]
    pub savings_estimated_turns: i64,
    #[serde(
        default,
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub savings_estimated_actual_spend: Decimal,
    // money out as a number, and the plain Option reader back, which takes the unit a buffered tagged enum gives a null
    #[serde(
        default,
        rename = "savings_estimated_classifier_cost",
        serialize_with = "serialize_optional_money"
    )]
    pub savings_classifier_cost: Option<Decimal>,
    #[serde(
        default,
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub savings_estimated_saved_spend: Decimal,
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub classifier_cost: Decimal,
    pub classifier_cost_recorded_turns: i64,
    #[serde(
        serialize_with = "serialize_money",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub session_seconds: Decimal,
}

// The comparison cohort: turns that ran without a recorded baseline cost are not part of it
#[must_use]
pub fn savings_cohort(
    turns: i64,
    estimated_turns: i64,
    spend: Decimal,
    saved_spend: Decimal,
) -> (Option<Decimal>, Option<Decimal>) {
    if turns > 0 && estimated_turns == 0 && saved_spend.is_zero() {
        return (None, None);
    }
    (Some(saved_spend), Some(spend.saturating_add(saved_spend)))
}

// A row relabelled for the savings comparison: only complexity routers with a baseline compare
#[must_use]
pub fn compared_row(row: &SessionAggRow) -> SessionAggRow {
    let (baseline_spend, _baseline_total) = savings_cohort(
        row.turns,
        row.savings_estimated_turns,
        row.spend,
        row.saved_spend,
    );
    let compared = row.router_type == "complexity" && baseline_spend.is_some();
    let classifier_cost = if compared {
        (row.classifier_cost_recorded_turns == row.turns).then_some(row.classifier_cost)
    } else {
        Some(Decimal::ZERO)
    };
    SessionAggRow {
        savings_estimated_turns: if compared { row.turns } else { 0 },
        savings_estimated_actual_spend: if compared { row.spend } else { Decimal::ZERO },
        savings_classifier_cost: classifier_cost,
        savings_estimated_saved_spend: if compared {
            row.saved_spend
        } else {
            Decimal::ZERO
        },
        ..row.clone()
    }
}
