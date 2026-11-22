use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::leg::ShadowLeg;
use super::results::ShadowEvalResult;
use super::{ShadowEvalModelError, ShadowEvalStatus};
use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection;

// ShadowEvalJobResponse: one job over independent target legs; status is derived, never stored
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowJob {
    pub job_id: String,
    pub targets: Vec<ShadowLeg>,
    pub router_names: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default = "default_direction")]
    pub direction: ShadowEvalDirection,
    #[serde(default)]
    pub baseline_model: Option<String>,
    pub judge_model: String,
    pub shadow_percentage: f64,
    pub created_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    #[serde(default)]
    pub stopped_by: Option<String>,
    #[serde(default)]
    pub judged_count: Option<i64>,
    #[serde(default)]
    pub error_count: Option<i64>,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub judge_spend: Option<Decimal>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub results: Option<ShadowEvalResult>,
}

impl ShadowJob {
    // the first router, kept for callers that predate router_names, so the two fields never disagree
    pub fn router_name(&self) -> Result<&str, ShadowEvalModelError> {
        self.router_names
            .first()
            .map(String::as_str)
            .ok_or(ShadowEvalModelError::NoRouterArms)
    }

    // stopped_by is the stop record; the window, every target's budget, then every target's stamp
    pub fn status(&self, now: DateTime<Utc>) -> Result<ShadowEvalStatus, ShadowEvalModelError> {
        if self.targets.is_empty() {
            return Err(ShadowEvalModelError::NoTargets);
        }
        if self.stopped_by.is_some() {
            return Ok(ShadowEvalStatus::Stopped);
        }
        if now >= self.ends_at {
            return Ok(ShadowEvalStatus::Completed);
        }
        if self.targets.iter().all(ShadowLeg::budget_spent) {
            return Ok(ShadowEvalStatus::Completed);
        }
        if self
            .targets
            .iter()
            .all(|target| target.stopped_at.is_some())
        {
            return Ok(ShadowEvalStatus::Stopped);
        }
        Ok(ShadowEvalStatus::Running)
    }

    // the one constructor of a job response: router_name and status are derived here and nowhere else
    pub fn response(&self, now: DateTime<Utc>) -> Result<ShadowJobResponse, ShadowEvalModelError> {
        Ok(ShadowJobResponse {
            status: self.status(now)?,
            router_name: self.router_name()?.to_owned(),
            job: self.clone(),
        })
    }
}

const fn default_direction() -> ShadowEvalDirection {
    ShadowEvalDirection::Forward
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ShadowJobResponse {
    #[serde(flatten)]
    pub job: ShadowJob,
    pub router_name: String,
    pub status: ShadowEvalStatus,
}
