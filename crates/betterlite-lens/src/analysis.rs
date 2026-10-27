use crate::models::decode::{DecodeError, decode};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::task::JoinError;

use crate::models::execution::{Coverage, Execution, ExecutionContent, TracePart};
use crate::models::finding::{Evidence, FindingDraft};
use crate::models::messages::{ModelRequest, ModelResult};
use crate::trace_store::TraceStoreError;
use garde::Validate;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

pub const ANALYSIS_CONCURRENCY: usize = 8;
pub trait Record: Serialize + DeserializeOwned + Send + Sync + Validate<Context = ()> {
    fn model_validate_json(bytes: &[u8]) -> Result<Self, DecodeError> {
        decode(bytes)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Observation {
    pub check_id: String,
    #[serde(default)]
    pub kind: ObservationKind,
    #[garde(length(chars, max = 2000))]
    pub summary: String,
    #[serde(default)]
    #[garde(length(max = 6), dive)]
    pub evidence: Vec<Evidence>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ObservationKind {
    #[default]
    Issue,
    Pattern,
}
impl Record for Observation {}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Extraction {
    #[serde(default)]
    #[garde(dive)]
    pub observations: Vec<Observation>,
    #[serde(default)]
    pub cannot_assess: bool,
}
impl Record for Extraction {}

#[derive(Clone, Debug, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct SpanRead {
    pub span_id: String,
    #[serde(default)]
    #[garde(range(min = 0))]
    pub offset: i64,
}
impl Record for SpanRead {}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct TraceReview {
    #[serde(default)]
    pub cannot_assess: bool,
    #[serde(default)]
    #[garde(dive)]
    pub observations: Vec<Observation>,
    #[serde(default)]
    #[garde(range(min = 0))]
    pub feedback_page: Option<i64>,
    #[serde(default)]
    #[garde(length(max = 2), dive)]
    pub reads: Vec<SpanRead>,
}
impl Record for TraceReview {}

#[derive(Clone, Debug, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Candidate {
    pub check_id: String,
    #[serde(default)]
    pub kind: ObservationKind,
    #[garde(length(chars, max = 160))]
    pub title: String,
    #[garde(length(chars, max = 2000))]
    pub hypothesis: String,
    pub execution_ids: Vec<String>,
    #[serde(default)]
    pub existing_finding_id: Option<String>,
}
impl Record for Candidate {}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Clusters {
    #[serde(default)]
    #[garde(dive)]
    pub candidates: Vec<Candidate>,
}
impl Record for Clusters {}

#[derive(Clone, Debug, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Decision {
    pub action: DecisionAction,
    #[serde(default)]
    #[garde(range(min = 0))]
    pub page: i64,
    #[serde(default)]
    pub execution_id: Option<String>,
    #[serde(default)]
    pub cursor: String,
    #[serde(default)]
    #[garde(range(min = 0))]
    pub offset: i64,
    #[serde(default)]
    #[garde(dive)]
    pub finding: Option<FindingDraft>,
}
impl Record for Decision {}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DecisionAction {
    Read,
    Observations,
    Catalog,
    Feedback,
    Submit,
    Inconclusive,
}

#[derive(Clone, Debug, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct FinalDecision {
    pub action: FinalAction,
    #[serde(default)]
    #[garde(dive)]
    pub finding: Option<FindingDraft>,
}
impl Record for FinalDecision {}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FinalAction {
    Submit,
    Inconclusive,
}

#[derive(Clone, Debug, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Examined {
    #[garde(dive)]
    pub execution: Execution,
    #[garde(dive)]
    pub observations: Vec<Observation>,
    pub parts: Vec<TracePart>,
    pub partial: bool,
    pub cannot_assess: bool,
}
impl Record for Examined {}
#[derive(Clone, Debug, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Investigation {
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(dive)]
    pub finding: Option<FindingDraft>,
    pub parts: Vec<TracePart>,
}
impl Record for Investigation {}

pub type ModelCall = Arc<dyn Fn(ModelRequest) -> ModelFuture + Send + Sync>;
pub type ModelFuture = Pin<Box<dyn Future<Output = Result<ModelResult, AnalysisError>> + Send>>;
pub type ReadContent = Arc<dyn Fn(String, String, i64) -> ReadFuture + Send + Sync>;
pub type ReadFuture = Pin<Box<dyn Future<Output = Result<ExecutionContent, AnalysisError>> + Send>>;
pub type ReportProgress = Arc<dyn Fn(String, Coverage) -> ProgressFuture + Send + Sync>;
pub type ProgressFuture = Pin<Box<dyn Future<Output = Result<(), AnalysisError>> + Send>>;

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error("model response is invalid: {0}")]
    Decode(#[from] DecodeError),
    #[error("analysis operation failed: {0}")]
    Join(#[from] JoinError),
    #[error("model response is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("structured response failed validation: {0}")]
    Validation(String),
    #[error("trace storage failed: {0}")]
    Store(#[from] TraceStoreError),
    #[error("callback failed: {0}")]
    Callback(String),
}

pub mod primitives;

#[path = "analysis/tests/concurrent_results.rs"]
mod concurrent_results;
mod tests;
