use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_inline_default::serde_inline_default;

use crate::models::execution::{Coverage, RunAssessment, Sample};
use crate::models::finding::Finding;
use crate::models::settings::{LensSettings, Scope};
use crate::models::validate::{RecordError, Validate};

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
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub id: String,
    #[serde(default)]
    pub status: JobStatus,
    #[serde_inline_default("Queued".to_owned())]
    pub stage: String,
    pub created_at: DateTime<Utc>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
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
    pub sample: Option<Sample>,
    #[serde(default, with = "rust_decimal::serde::float")]
    pub cost: Decimal,
    // absent until the job reports results, which differs from reporting none
    #[serde(default)]
    pub findings: Option<Vec<Finding>>,
    #[serde(default)]
    pub assessments: Vec<RunAssessment>,
}

impl Validate for Job {
    fn validate(&self) -> Result<(), RecordError> {
        self.settings.validate()?;
        self.sample.iter().try_for_each(Validate::validate)?;
        self.findings
            .iter()
            .flatten()
            .try_for_each(Validate::validate)
    }
}

#[serde_inline_default]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Lens {
    pub id: String,
    pub scope: Scope,
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
    pub jobs: Vec<Job>,
    #[serde(default)]
    pub findings: Vec<Finding>,
    pub budget_month: String,
    #[serde(default, with = "rust_decimal::serde::float")]
    pub spent: Decimal,
}

impl Validate for Lens {
    fn validate(&self) -> Result<(), RecordError> {
        self.settings.validate()?;
        self.jobs.iter().try_for_each(Validate::validate)?;
        self.findings.iter().try_for_each(Validate::validate)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Worker {
    // absent for workers registered before analysis keys existed
    #[serde(default)]
    pub analysis_key_id: Option<String>,
    pub id: String,
    pub name: String,
    pub scope: Scope,
    pub last_seen: DateTime<Utc>,
    #[serde(default)]
    pub revoked: bool,
}

impl Validate for Worker {
    fn validate(&self) -> Result<(), RecordError> {
        let malformed = self.analysis_key_id.as_ref().is_some_and(|key| {
            key.len() != 64
                || !key
                    .bytes()
                    .all(|byte| matches!(byte, b'a'..=b'f' | b'0'..=b'9'))
        });
        if malformed {
            Err(RecordError::Pattern {
                field: "analysis_key_id",
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerCreated {
    pub worker: Worker,
    pub token: SecretString,
}

impl Validate for WorkerCreated {
    fn validate(&self) -> Result<(), RecordError> {
        self.worker.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LensList {
    pub lenses: Vec<Lens>,
    pub workers: Vec<Worker>,
    pub tracing_enabled: bool,
}

impl Validate for LensList {
    fn validate(&self) -> Result<(), RecordError> {
        self.lenses.iter().try_for_each(Validate::validate)?;
        self.workers.iter().try_for_each(Validate::validate)
    }
}
