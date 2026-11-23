use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use crate::shadow_eval_models::ShadowEvalTargetType;

// one LiteLLM_ShadowEvalJob row: a target's leg of a job, with the config its group shares
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LegRow {
    pub id: String,
    pub group_id: String,
    pub target_type: ShadowEvalTargetType,
    pub target_id: String,
    pub router_name: String,
    #[serde(default)]
    pub router_names: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    pub direction: ShadowEvalDirection,
    #[serde(default)]
    pub baseline_model: Option<String>,
    pub judge_model: String,
    pub shadow_percentage: f64,
    pub max_turns: i64,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub max_budget: Option<Decimal>,
    pub created_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    #[serde(default)]
    pub stopped_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub stopped_by: Option<String>,
}

impl LegRow {
    // rows from before router_names existed hold the job's router set in router_name alone
    #[must_use]
    pub fn arm_router_names(&self) -> Vec<String> {
        if self.router_names.is_empty() {
            return vec![self.router_name.clone()];
        }
        self.router_names.clone()
    }
}
