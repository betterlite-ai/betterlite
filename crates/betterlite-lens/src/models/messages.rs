use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::models::execution::{Coverage, RunAssessment};
use crate::models::finding::{Finding, FindingDraft, FindingStatus};
use crate::models::job::Job;
use crate::models::settings::LensSettings;
use crate::models::validate::{RecordError, Validate, check_length};

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunRequest {
    // either absent field falls back to the lens's own settings
    pub settings: Option<LensSettings>,
    pub lookback_hours: Option<u32>,
}

impl Validate for RunRequest {
    fn validate(&self) -> Result<(), RecordError> {
        if self
            .lookback_hours
            .is_some_and(|hours| !(1..=8760).contains(&hours))
        {
            return Err(RecordError::Range {
                field: "lookback_hours",
            });
        }
        self.settings.iter().try_for_each(Validate::validate)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FindingUpdate {
    pub status: FindingStatus,
    #[serde(default)]
    pub reason: String,
}

impl Validate for FindingUpdate {
    fn validate(&self) -> Result<(), RecordError> {
        check_length("reason", &self.reason, 0, 2000)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub lens_id: String,
    pub job: Job,
    pub findings: Vec<Finding>,
}

impl Validate for Claim {
    fn validate(&self) -> Result<(), RecordError> {
        self.job.validate()?;
        self.findings.iter().try_for_each(Validate::validate)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub stage: String,
    #[serde(default)]
    pub coverage: Coverage,
}

impl Validate for Progress {
    fn validate(&self) -> Result<(), RecordError> {
        check_length("stage", &self.stage, 0, 100)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JobResult {
    #[serde(default)]
    pub assessments: Vec<RunAssessment>,
    #[serde(default)]
    pub findings: Vec<FindingDraft>,
    pub coverage: Coverage,
    #[serde(default)]
    pub error: String,
}

impl Validate for JobResult {
    fn validate(&self) -> Result<(), RecordError> {
        self.findings.iter().try_for_each(Validate::validate)?;
        check_length("error", &self.error, 0, 1000)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Purpose {
    Extract,
    Cluster,
    Investigate,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequest {
    pub prompt: String,
    pub purpose: Purpose,
}

impl Validate for ModelRequest {
    fn validate(&self) -> Result<(), RecordError> {
        check_length("prompt", &self.prompt, 1, 100_000)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelResult {
    pub content: String,
    #[serde(with = "rust_decimal::serde::float")]
    pub cost: Decimal,
}

impl Validate for ModelResult {}
