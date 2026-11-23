use betterlite_server::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use betterlite_server::shadow_eval_models::ShadowEvalStatus;
use betterlite_server::shadow_eval_models::ShadowEvalTargetType;
use betterlite_server::shadow_eval_models::job::ShadowJobResponse;
use betterlite_server::shadow_eval_models::leg::ShadowLeg;
use betterlite_server::shadow_result_aggregation::legs::LegRow;
use betterlite_server::shadow_result_aggregation::targets::target_type_name;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct OutputFields {
    pub(super) steps: Vec<StepOutcome>,
    pub(super) jobs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub(super) enum StepOutcome {
    Succeeded { status: u16, job: Box<JobView> },
    Failed { status: u16, detail: String },
    Accepted,
    Invalid,
    OverLimit { bytes: usize, limit: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct JobView {
    pub(super) job_id: String,
    pub(super) router_name: String,
    pub(super) router_names: Vec<String>,
    pub(super) models: Vec<String>,
    pub(super) direction: ShadowEvalDirection,
    pub(super) baseline_model: Option<String>,
    pub(super) judge_model: String,
    pub(super) shadow_percentage: String,
    pub(super) created_at_millis: i64,
    pub(super) ends_at_millis: i64,
    pub(super) stopped_by: Option<String>,
    pub(super) status: String,
    pub(super) targets: Vec<TargetView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TargetView {
    pub(super) target_type: ShadowEvalTargetType,
    pub(super) target_id: String,
    pub(super) max_turns: i64,
    pub(super) max_budget: Option<String>,
    pub(super) stopped_at_millis: Option<i64>,
    pub(super) attempt_count: Option<i64>,
    pub(super) spend: Option<String>,
    pub(super) target_alias: Option<String>,
    pub(super) key_name: Option<String>,
}

const fn status_text(status: ShadowEvalStatus) -> &'static str {
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
        max_budget: leg.max_budget.map(|amount| format!("{amount:.15}")),
        stopped_at_millis: leg.stopped_at.map(|stamp| stamp.timestamp_millis()),
        attempt_count: leg.attempt_count,
        spend: leg.spend.map(|amount| format!("{amount:.6}")),
        target_alias: leg.target_alias,
        key_name: leg.key_name,
    }
}

impl JobView {
    pub(super) fn of(response: ShadowJobResponse) -> Self {
        let job = response.job;
        Self {
            job_id: job.job_id,
            router_name: response.router_name,
            router_names: job.router_names,
            models: job.models,
            direction: job.direction,
            baseline_model: job.baseline_model,
            judge_model: job.judge_model,
            shadow_percentage: format!("{:.2}", job.shadow_percentage),
            created_at_millis: job.created_at.timestamp_millis(),
            ends_at_millis: job.ends_at.timestamp_millis(),
            stopped_by: job.stopped_by,
            status: status_text(response.status).to_owned(),
            targets: job.targets.into_iter().map(target_view).collect(),
        }
    }
}

// one stored leg as a single exact cell, so both sides can sort and compare it without an enum order
pub(super) fn leg_state(leg: &LegRow) -> String {
    let stopped = leg
        .stopped_at
        .map_or_else(String::new, |stamp| stamp.timestamp_millis().to_string());
    let actor = leg.stopped_by.as_deref().unwrap_or_default();
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{:.2}|{stopped}|{actor}",
        leg.id,
        leg.group_id,
        target_type_name(leg.target_type),
        leg.target_id,
        leg.router_names.join(","),
        leg.models.join(","),
        leg.direction.as_str(),
        leg.judge_model,
        leg.max_turns,
        leg.shadow_percentage,
    )
}
