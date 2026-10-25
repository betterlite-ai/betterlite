use garde::Validate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::models::execution::{Coverage, RunAssessment};
use crate::models::finding::{Finding, FindingDraft, FindingStatus};
use crate::models::job::Job;
use crate::models::settings::LensSettings;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct RunRequest {
    // either absent field falls back to the lens's own settings
    #[garde(dive)]
    pub settings: Option<LensSettings>,
    #[garde(range(min = 1, max = 8760))]
    pub lookback_hours: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct FindingUpdate {
    pub status: FindingStatus,
    #[serde(default)]
    #[garde(length(chars, max = 2000))]
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Claim {
    pub lens_id: String,
    #[garde(dive)]
    pub job: Job,
    #[garde(dive)]
    pub findings: Vec<Finding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Progress {
    #[garde(length(chars, max = 100))]
    pub stage: String,
    #[serde(default)]
    pub coverage: Coverage,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct JobResult {
    #[serde(default)]
    pub assessments: Vec<RunAssessment>,
    #[serde(default)]
    #[garde(dive)]
    pub findings: Vec<FindingDraft>,
    pub coverage: Coverage,
    #[serde(default)]
    #[garde(length(chars, max = 1000))]
    pub error: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Purpose {
    Extract,
    Cluster,
    Investigate,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct ModelRequest {
    #[garde(length(chars, min = 1, max = 100_000))]
    pub prompt: String,
    pub purpose: Purpose,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct ModelResult {
    pub content: String,
    #[serde(with = "rust_decimal::serde::float")]
    pub cost: Decimal,
}
