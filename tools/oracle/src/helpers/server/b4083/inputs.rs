use std::collections::BTreeMap;

use betterlite_db::shadow_tables::rows::ShadowJobRow;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    ValidateJob(Box<ValidateCall>),
    LoadJobs(Box<LoadCall>),
    RecordAttempt(Box<AttemptCall>),
}

#[derive(Deserialize)]
pub(crate) struct ValidateCall {
    pub(crate) arm_router: String,
    pub(crate) rows: Vec<JobInput>,
}

#[derive(Deserialize)]
pub(crate) struct LoadCall {
    pub(crate) rows: Vec<JobInput>,
    #[serde(default)]
    pub(crate) stats: Vec<StatInput>,
    #[serde(default)]
    pub(crate) metadata: BTreeMap<String, Value>,
    pub(crate) model_group: String,
    #[serde(default)]
    pub(crate) aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) hook_calls: usize,
}

#[derive(Deserialize)]
pub(crate) struct AttemptCall {
    pub(crate) job: JobInput,
    pub(crate) row: RowInput,
}

#[derive(Deserialize)]
pub(crate) struct JobInput {
    pub(crate) id: String,
    pub(crate) target_type: String,
    pub(crate) target_id: String,
    #[serde(default)]
    pub(crate) stopped_at: Option<DateTime<Utc>>,
    pub(crate) router_name: String,
    #[serde(default)]
    pub(crate) router_names: Vec<String>,
    #[serde(default)]
    pub(crate) models: Vec<String>,
    pub(crate) direction: String,
    #[serde(default)]
    pub(crate) baseline_model: Option<String>,
    pub(crate) shadow_percentage: f64,
    pub(crate) judge_model: String,
    pub(crate) max_turns: i64,
    #[serde(default)]
    pub(crate) max_budget: Option<Decimal>,
    pub(crate) ends_at: DateTime<Utc>,
}

impl JobInput {
    pub(crate) fn to_row(&self) -> ShadowJobRow {
        ShadowJobRow {
            id: self.id.clone(),
            target_type: self.target_type.clone(),
            target_id: self.target_id.clone(),
            router_name: self.router_name.clone(),
            router_names: self.router_names.clone(),
            models: self.models.clone(),
            direction: self.direction.clone(),
            baseline_model: self.baseline_model.clone(),
            shadow_percentage: self.shadow_percentage,
            judge_model: self.judge_model.clone(),
            max_turns: self.max_turns,
            max_budget: self.max_budget,
            ends_at: self.ends_at,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct StatInput {
    pub(crate) job_id: String,
    pub(crate) attempts: i64,
    pub(crate) spend: Decimal,
}

#[derive(Deserialize)]
pub(crate) struct RowInput {
    pub(crate) request_id: String,
    pub(crate) router_name: String,
    pub(crate) outcome: String,
    #[serde(default)]
    pub(crate) tier: Option<String>,
    pub(crate) real_model: String,
    #[serde(default)]
    pub(crate) shadow_model: Option<String>,
    #[serde(default)]
    pub(crate) confidence: Option<Decimal>,
    pub(crate) judge_cost: Decimal,
    pub(crate) shadow_cost: Decimal,
    pub(crate) shadow_classifier_cost: Decimal,
    pub(crate) real_cost: Decimal,
    pub(crate) real_classifier_cost: Decimal,
    pub(crate) real_cache_hit: bool,
    #[serde(default)]
    pub(crate) error: Option<String>,
}
