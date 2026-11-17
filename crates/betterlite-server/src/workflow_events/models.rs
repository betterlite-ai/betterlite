use serde::{Deserialize, Serialize};

use crate::workflow_runs::models::{JsonObject, WorkflowEventRow, WorkflowMessageRow};

#[derive(Debug, Deserialize)]
pub struct WorkflowEventCreateRequest {
    pub event_type: String,
    pub step_name: String,
    pub data: Option<JsonObject>,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowMessageCreateRequest {
    pub role: String,
    pub content: String,
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventCreateData {
    pub run_id: String,
    pub event_type: String,
    pub step_name: String,
    pub sequence_number: i32,
    pub data: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageCreateData {
    pub run_id: String,
    pub role: String,
    pub content: String,
    pub sequence_number: i32,
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EventListResponse {
    pub events: Vec<WorkflowEventRow>,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct MessageListResponse {
    pub messages: Vec<WorkflowMessageRow>,
    pub count: usize,
}
