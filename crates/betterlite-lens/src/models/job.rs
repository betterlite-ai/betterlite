use chrono::{DateTime, Utc};
use garde::Validate;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_inline_default::serde_inline_default;

use crate::models::execution::{Coverage, RunAssessment, Sample};
use crate::models::finding::Finding;
use crate::models::settings::{LensSettings, Scope};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    #[default]
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[serde_inline_default]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Job {
    pub id: String,
    #[serde(default)]
    pub status: JobStatus,
    #[serde_inline_default("Queued".to_owned())]
    pub stage: String,
    pub created_at: DateTime<Utc>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[garde(dive)]
    pub settings: LensSettings,
    pub revision: i64,
    // the next four stay absent until a worker claims or finishes the job
    #[serde(default)]
    pub worker_id: Option<String>,
    #[serde(default)]
    pub lease_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub attempts: i64,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub coverage: Coverage,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    #[garde(dive)]
    pub sample: Option<Sample>,
    #[serde(default, with = "rust_decimal::serde::float")]
    pub cost: Decimal,
    // absent until the job reports results, which differs from reporting none
    #[serde(default)]
    #[garde(dive)]
    pub findings: Option<Vec<Finding>>,
    #[serde(default)]
    pub assessments: Vec<RunAssessment>,
}

#[serde_inline_default]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Lens {
    pub id: String,
    pub scope: Scope,
    #[garde(dive)]
    pub settings: LensSettings,
    #[serde_inline_default(1)]
    pub revision: i64,
    #[serde(default)]
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub next_run_at: DateTime<Utc>,
    #[serde(default)]
    pub last_scan_at: Option<DateTime<Utc>>,
    #[serde(default)]
    #[garde(dive)]
    pub jobs: Vec<Job>,
    #[serde(default)]
    #[garde(dive)]
    pub findings: Vec<Finding>,
    pub budget_month: String,
    #[serde(default, with = "rust_decimal::serde::float")]
    pub spent: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Worker {
    // absent for workers registered before analysis keys existed
    #[serde(default)]
    #[garde(pattern(r"^[a-f0-9]{64}$"))]
    pub analysis_key_id: Option<String>,
    pub id: String,
    pub name: String,
    pub scope: Scope,
    pub last_seen: DateTime<Utc>,
    #[serde(default)]
    pub revoked: bool,
}

#[derive(Debug, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct WorkerCreated {
    #[garde(dive)]
    pub worker: Worker,
    pub token: SecretString,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct LensList {
    #[garde(dive)]
    pub lenses: Vec<Lens>,
    #[garde(dive)]
    pub workers: Vec<Worker>,
    pub tracing_enabled: bool,
}
