use betterlite_server::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use betterlite_server::shadow_eval_lifecycle::store::{LegRow, target_type_text};
use betterlite_server::shadow_eval_models::ShadowEvalStatus;
use betterlite_server::shadow_eval_models::ShadowEvalTargetType;
use betterlite_server::shadow_eval_models::job::{ShadowJobResponse, ShadowLeg};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    pub steps: Vec<StepOutcome>,
    pub jobs: Vec<JobState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum StepOutcome {
    Succeeded { status: u16, job: JobView },
    Failed { status: u16, detail: String },
    Accepted,
    Invalid,
    OverLimit { bytes: usize, limit: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobView {
    pub job_id: String,
    pub router_name: String,
    pub router_names: Vec<String>,
    pub models: Vec<String>,
    pub direction: ShadowEvalDirection,
    pub baseline_model: Option<String>,
    pub judge_model: String,
    pub shadow_percentage: String,
    pub created_at_millis: i64,
    pub ends_at_millis: i64,
    pub stopped_by: Option<String>,
    pub status: String,
    pub targets: Vec<TargetView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetView {
    pub target_type: ShadowEvalTargetType,
    pub target_id: String,
    pub max_turns: i64,
    pub max_budget: Option<String>,
    pub stopped_at_millis: Option<i64>,
    pub attempt_count: Option<i64>,
    pub spend: Option<String>,
    pub target_alias: Option<String>,
    pub key_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobState {
    pub id: String,
    pub group_id: String,
    pub target_type: ShadowEvalTargetType,
    pub target_id: String,
    pub router_name: String,
    pub router_names: Vec<String>,
    pub models: Vec<String>,
    pub direction: ShadowEvalDirection,
    pub baseline_model: Option<String>,
    pub judge_model: String,
    pub shadow_percentage: String,
    pub max_turns: i64,
    pub max_budget: Option<String>,
    pub created_at_millis: i64,
    pub ends_at_millis: i64,
    pub stopped_at_millis: Option<i64>,
    pub stopped_by: Option<String>,
}

fn percentage(value: f64) -> String {
    format!("{value:.2}")
}

fn budget(value: Option<Decimal>) -> Option<String> {
    value.map(|amount| format!("{amount:.15}"))
}

fn spend(value: Option<Decimal>) -> Option<String> {
    value.map(|amount| format!("{amount:.6}"))
}

fn status_text(status: ShadowEvalStatus) -> &'static str {
    match status {
        ShadowEvalStatus::Running => "running",
        ShadowEvalStatus::Completed => "completed",
        ShadowEvalStatus::Stopped => "stopped",
    }
}

fn target_view(leg: ShadowLeg) -> TargetView {
    TargetView {
        target_type: leg.target_type,
        target_id: leg.target_id,
        max_turns: leg.max_turns,
        max_budget: budget(leg.max_budget),
        stopped_at_millis: leg.stopped_at.map(|stamp| stamp.timestamp_millis()),
        attempt_count: leg.attempt_count,
        spend: spend(leg.spend),
        target_alias: leg.target_alias,
        key_name: leg.key_name,
    }
}

impl JobView {
    pub(crate) fn of(response: ShadowJobResponse) -> Self {
        let job = response.job;
        Self {
            job_id: job.job_id,
            router_name: response.router_name,
            router_names: job.router_names,
            models: job.models,
            direction: job.direction,
            baseline_model: job.baseline_model,
            judge_model: job.judge_model,
            shadow_percentage: percentage(job.shadow_percentage),
            created_at_millis: job.created_at.timestamp_millis(),
            ends_at_millis: job.ends_at.timestamp_millis(),
            stopped_by: job.stopped_by,
            status: status_text(response.status).to_owned(),
            targets: job.targets.into_iter().map(target_view).collect(),
        }
    }
}

impl JobState {
    pub(crate) fn of(leg: &LegRow) -> Self {
        Self {
            id: leg.id.clone(),
            group_id: leg.group_id.clone(),
            target_type: leg.target_type,
            target_id: leg.target_id.clone(),
            router_name: leg.router_name.clone(),
            router_names: leg.router_names.clone(),
            models: leg.models.clone(),
            direction: leg.direction,
            baseline_model: leg.baseline_model.clone(),
            judge_model: leg.judge_model.clone(),
            shadow_percentage: percentage(leg.shadow_percentage),
            max_turns: leg.max_turns,
            max_budget: budget(leg.max_budget),
            created_at_millis: leg.created_at.timestamp_millis(),
            ends_at_millis: leg.ends_at.timestamp_millis(),
            stopped_at_millis: leg.stopped_at.map(|stamp| stamp.timestamp_millis()),
            stopped_by: leg.stopped_by.clone(),
        }
    }
}

impl PartialOrd for JobState {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for JobState {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            self.group_id.as_str(),
            target_type_text(self.target_type),
            self.target_id.as_str(),
            self.id.as_str(),
        )
            .cmp(&(
                other.group_id.as_str(),
                target_type_text(other.target_type),
                other.target_id.as_str(),
                other.id.as_str(),
            ))
    }
}
