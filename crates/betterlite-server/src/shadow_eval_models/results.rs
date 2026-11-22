use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// ShadowEvalSlice: the judge outcomes of one router tier, served model, router arm or target
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowEvalSlice {
    pub group: String,
    pub turn_count: i64,
    pub real_win_rate_pct: f64,
    pub shadow_win_rate_pct: f64,
    pub tie_rate_pct: f64,
    pub avg_judge_confidence: f64,
    #[serde(default, with = "rust_decimal::serde::float")]
    pub real_spend: Decimal,
    #[serde(default, with = "rust_decimal::serde::float")]
    pub shadow_spend: Decimal,
    #[serde(default)]
    pub cache_hit_turns: i64,
}

// ShadowEvalResult: one job's stratified verdicts, populated by the detail endpoint only
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowEvalResult {
    pub by_tier: Vec<ShadowEvalSlice>,
    pub by_current_model: Vec<ShadowEvalSlice>,
    #[serde(default)]
    pub by_router: Vec<ShadowEvalSlice>,
    pub overall_shadow_win_rate_pct: f64,
    pub overall_tie_rate_pct: f64,
    #[serde(default, with = "rust_decimal::serde::float")]
    pub sampled_real_spend: Decimal,
    #[serde(default, with = "rust_decimal::serde::float")]
    pub sampled_shadow_spend: Decimal,
    #[serde(default)]
    pub not_sampled_count: Option<i64>,
    #[serde(default)]
    pub unjudgeable_count: Option<i64>,
    #[serde(default)]
    pub shed_count: Option<i64>,
    #[serde(default)]
    pub withheld_count: Option<i64>,
}
