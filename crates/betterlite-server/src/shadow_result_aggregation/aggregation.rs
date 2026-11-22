use std::cmp::Reverse;
use std::ops::{Div as _, Mul as _};

use num_traits::ToPrimitive as _;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::error::ShadowResultsError;
use crate::shadow_eval_models::results::ShadowEvalSlice;

// one row of LiteLLM's _ATTEMPT_AGG queries: the judged turns of one tier, model, leg or router
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AttemptAggRow {
    #[serde(rename = "grp")]
    pub group: String,
    pub turn_count: i64,
    pub real_wins: i64,
    pub shadow_wins: i64,
    pub ties: i64,
    pub avg_confidence: Option<f64>,
    pub real_spend: Decimal,
    pub shadow_spend: Decimal,
    pub cache_hit_turns: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupBy {
    Tier,
    Model,
    Leg,
    Router,
}

// Python's round(value, places): ties to even over the exact value of the binary double
pub fn round_to(
    value: f64,
    places: u32,
    name: &'static str,
) -> Result<Decimal, ShadowResultsError> {
    let exact =
        Decimal::from_f64_retain(value).ok_or(ShadowResultsError::Unrepresentable { name })?;
    Ok(exact.round_dp(places))
}

// _pct_of: the denominator is the judged-turn count, and an empty or negative slice reads 0.0
pub fn pct_of(numerator: i64, denominator: i64) -> Result<f64, ShadowResultsError> {
    if denominator <= 0 {
        return Ok(0.0_f64);
    }
    let numerator = numerator
        .to_f64()
        .ok_or(ShadowResultsError::Unrepresentable { name: "numerator" })?;
    let denominator = denominator
        .to_f64()
        .ok_or(ShadowResultsError::Unrepresentable {
            name: "denominator",
        })?;
    let rate = Div::div(Mul::mul(100.0_f64, numerator), denominator);
    Ok(round_to(rate, 1, "rate")?.as_f64())
}

// _slices: one slice per group, the busiest group first, ties keeping the read's own order
pub fn slices(rows: &[AttemptAggRow]) -> Result<Vec<ShadowEvalSlice>, ShadowResultsError> {
    let mut ordered: Vec<&AttemptAggRow> = rows.iter().collect();
    ordered.sort_by_key(|row| Reverse(row.turn_count));
    ordered.into_iter().map(slice).collect()
}

fn slice(row: &AttemptAggRow) -> Result<ShadowEvalSlice, ShadowResultsError> {
    Ok(ShadowEvalSlice {
        group: row.group.clone(),
        turn_count: row.turn_count,
        real_win_rate_pct: pct_of(row.real_wins, row.turn_count)?,
        shadow_win_rate_pct: pct_of(row.shadow_wins, row.turn_count)?,
        tie_rate_pct: pct_of(row.ties, row.turn_count)?,
        avg_judge_confidence: round_to(
            row.avg_confidence.unwrap_or(0.0_f64),
            3,
            "avg_judge_confidence",
        )?
        .as_f64(),
        real_spend: row.real_spend,
        shadow_spend: row.shadow_spend,
        cache_hit_turns: row.cache_hit_turns,
    })
}
