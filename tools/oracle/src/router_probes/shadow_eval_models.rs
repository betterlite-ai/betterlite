use betterlite_server::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use betterlite_server::shadow_eval_models::job::ShadowJob;
use betterlite_server::shadow_eval_models::leg::ShadowLeg;
use betterlite_server::shadow_eval_models::{ShadowEvalStatus, ShadowEvalTargetType};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// the lifecycle fields a job response derives from, typed Eq for the probe's operation enum
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub jobs: Vec<Job>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub job_id: String,
    pub router_names: Vec<String>,
    pub ends_at: String,
    #[serde(default)]
    pub stopped_by: Option<String>,
    pub targets: Vec<Leg>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Leg {
    pub target_type: ShadowEvalTargetType,
    pub target_id: String,
    pub max_turns: i64,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub max_budget: Option<Decimal>,
    #[serde(default)]
    pub attempt_count: Option<i64>,
    #[serde(default)]
    pub stopped_at: Option<String>,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub spend: Option<Decimal>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub responses: Vec<Response>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub job_id: String,
    pub router_name: String,
    pub status: ShadowEvalStatus,
    pub budget_spent: Vec<bool>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let responses = input
        .jobs
        .iter()
        .map(response)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ProbeOutput::ShadowEvalModels(Output { responses }))
}

fn response(job: &Job) -> Result<Response, OperationError> {
    let model = shadow_job(job)?;
    let built = model
        .response(Utc::now())
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(Response {
        job_id: built.job.job_id.clone(),
        router_name: built.router_name.clone(),
        status: built.status,
        budget_spent: model.targets.iter().map(ShadowLeg::budget_spent).collect(),
    })
}

fn shadow_job(job: &Job) -> Result<ShadowJob, OperationError> {
    let ends_at = time(&job.ends_at)?;
    let targets = job
        .targets
        .iter()
        .map(target)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ShadowJob {
        job_id: job.job_id.clone(),
        targets,
        router_names: job.router_names.clone(),
        models: Vec::new(),
        direction: ShadowEvalDirection::Forward,
        baseline_model: None,
        judge_model: String::new(),
        shadow_percentage: 0.0,
        created_at: ends_at,
        ends_at,
        stopped_by: job.stopped_by.clone(),
        judged_count: None,
        error_count: None,
        judge_spend: None,
        last_error: None,
        results: None,
    })
}

fn target(leg: &Leg) -> Result<ShadowLeg, OperationError> {
    Ok(ShadowLeg {
        target_type: leg.target_type,
        target_id: leg.target_id.clone(),
        max_turns: leg.max_turns,
        max_budget: leg.max_budget,
        stopped_at: leg.stopped_at.as_deref().map(time).transpose()?,
        attempt_count: leg.attempt_count,
        spend: leg.spend,
        verdicts: None,
        target_alias: None,
        key_name: None,
    })
}

fn time(text: &str) -> Result<DateTime<Utc>, OperationError> {
    text.parse()
        .map_err(|error| invalid(&format!("{text} is not an RFC 3339 time: {error}")))
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
