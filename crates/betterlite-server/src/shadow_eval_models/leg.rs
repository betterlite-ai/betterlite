use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::ShadowEvalTargetType;
use super::results::ShadowEvalSlice;

// ShadowEvalJobTargetResponse: one target's leg of a job, with its own budget and stop state
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowLeg {
    pub target_type: ShadowEvalTargetType,
    pub target_id: String,
    pub max_turns: i64,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub max_budget: Option<Decimal>,
    #[serde(default)]
    pub stopped_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub attempt_count: Option<i64>,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub spend: Option<Decimal>,
    #[serde(default)]
    pub verdicts: Option<ShadowEvalSlice>,
    #[serde(default)]
    pub target_alias: Option<String>,
    #[serde(default)]
    pub key_name: Option<String>,
}

impl ShadowLeg {
    // the target's own budget ran out, by spend or by the turn valve; a sibling's state never enters it
    #[must_use]
    pub fn budget_spent(&self) -> bool {
        let over_spend = self
            .max_budget
            .zip(self.spend)
            .is_some_and(|(budget, spend)| spend >= budget);
        over_spend
            || self
                .attempt_count
                .is_some_and(|attempts| attempts >= self.max_turns)
    }
}
