use serde::{Deserialize, Serialize};

use crate::models::settings::MetadataFilter;
use crate::models::validate::{RecordError, Validate};

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Coverage {
    pub eligible: i64,
    pub selected: i64,
    pub screened: i64,
    pub investigated: i64,
    pub inconclusive: i64,
    pub grouping_batches: i64,
    pub grouped_batches: i64,
    pub candidates: i64,
    pub partial: i64,
    pub unassessable: i64,
}

impl Validate for Coverage {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionSource {
    Traces,
    Requests,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Execution {
    pub id: String,
    pub source: ExecutionSource,
    pub trace_id: String,
    #[serde(default)]
    pub trace_ref: String,
    pub team_id: String,
    pub name: String,
    pub start_time: String,
    pub span_count: i64,
    #[serde(default)]
    pub root_seen: bool,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub metadata: Vec<MetadataFilter>,
}

impl Validate for Execution {
    fn validate(&self) -> Result<(), RecordError> {
        self.metadata.iter().try_for_each(Validate::validate)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TracePart {
    pub execution_id: String,
    pub span_id: String,
    #[serde(default)]
    pub parent_span_id: String,
    pub name: String,
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub truncated: bool,
}

impl Validate for TracePart {}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionContent {
    pub execution: Execution,
    pub parts: Vec<TracePart>,
    // absent once the last page is read
    #[serde(default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub partial: bool,
}

impl Validate for ExecutionContent {
    fn validate(&self) -> Result<(), RecordError> {
        self.execution.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub executions: Vec<Execution>,
    pub eligible: i64,
    #[serde(default)]
    pub selected: i64,
    // absent once the last page is read
    #[serde(default)]
    pub next_offset: Option<i64>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

impl Validate for Sample {
    fn validate(&self) -> Result<(), RecordError> {
        self.executions.iter().try_for_each(Validate::validate)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunAssessment {
    pub execution_id: String,
    #[serde(default)]
    pub issue_checks: Vec<String>,
    #[serde(default)]
    pub pattern_checks: Vec<String>,
    #[serde(default)]
    pub cannot_assess: bool,
}

impl Validate for RunAssessment {}
