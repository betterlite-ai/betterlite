use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::workflow_runs::error::RunsError;

pub const MAX_SEQUENCE_RETRIES: u8 = 5;

pub const EVENT_STATUS_MAP: [(&str, WorkflowRunStatus); 4] = [
    ("step.started", WorkflowRunStatus::Running),
    ("step.failed", WorkflowRunStatus::Failed),
    ("hook.waiting", WorkflowRunStatus::Paused),
    ("hook.received", WorkflowRunStatus::Running),
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, strum::Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum WorkflowRunStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JsonObject(
    #[serde(
        deserialize_with = "serde_with::As::<serde_with::MapPreventDuplicates<serde_with::Same, serde_with::Same>>::deserialize"
    )]
    pub BTreeMap<String, JsonData>,
);

#[derive(Debug, Deserialize)]
pub struct WorkflowRunCreateRequest {
    pub workflow_type: String,
    pub input: Option<JsonObject>,
    pub metadata: Option<JsonObject>,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowRunUpdateRequest {
    pub status: Option<WorkflowRunStatus>,
    pub output: Option<JsonObject>,
    pub metadata: Option<JsonObject>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunCreateData {
    pub workflow_type: String,
    pub created_by: Option<String>,
    pub input: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RunUpdateData {
    pub status: Option<WorkflowRunStatus>,
    pub output: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StatusFilter {
    Equals(String),
    In(Vec<String>),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RunWhere {
    pub workflow_type: Option<String>,
    pub status: Option<StatusFilter>,
    pub created_by: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceTable {
    Events,
    Messages,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SequenceRow {
    pub sequence_number: i32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkflowRunRow {
    pub run_id: String,
    pub session_id: String,
    pub workflow_type: String,
    pub status: String,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub input: Option<JsonData>,
    pub output: Option<JsonData>,
    pub metadata: Option<JsonData>,
    pub events: Option<Vec<WorkflowEventRow>>,
    pub messages: Option<Vec<WorkflowMessageRow>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkflowEventRow {
    pub event_id: String,
    pub run_id: String,
    pub event_type: String,
    pub step_name: String,
    pub sequence_number: i32,
    pub data: Option<JsonData>,
    pub created_at: DateTime<Utc>,
    pub run: Option<Box<WorkflowRunRow>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkflowMessageRow {
    pub message_id: String,
    pub run_id: String,
    pub role: String,
    pub content: String,
    pub sequence_number: i32,
    pub session_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub run: Option<Box<WorkflowRunRow>>,
}

#[derive(Debug, Serialize)]
pub struct RunListResponse {
    pub runs: Vec<WorkflowRunRow>,
    pub count: usize,
}

fn json_column(object: Option<&JsonObject>) -> Result<Option<String>, RunsError> {
    let Some(value) = object else {
        return Ok(None);
    };
    Ok(Some(serde_json::to_string(value)?))
}

impl RunCreateData {
    pub fn from_request(
        request: &WorkflowRunCreateRequest,
        created_by: Option<String>,
    ) -> Result<Self, RunsError> {
        let input = json_column(request.input.as_ref())?;
        let metadata = json_column(request.metadata.as_ref())?;
        Ok(Self {
            workflow_type: request.workflow_type.clone(),
            created_by,
            input,
            metadata,
        })
    }
}

impl RunUpdateData {
    pub fn from_request(request: &WorkflowRunUpdateRequest) -> Result<Self, RunsError> {
        Ok(Self {
            status: request.status,
            output: json_column(request.output.as_ref())?,
            metadata: json_column(request.metadata.as_ref())?,
        })
    }
}
