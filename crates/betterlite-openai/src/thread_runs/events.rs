use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::thread_runs::records::{IncompleteDetails, LastError, RunUsage};

// Message and step events keep every declared SDK field, so an omitted one stays null, and hold extras.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunMessage {
    pub id: String,
    pub assistant_id: Option<String>,
    pub attachments: Option<Value>,
    pub completed_at: Option<i64>,
    pub content: Vec<Value>,
    pub created_at: i64,
    pub incomplete_at: Option<i64>,
    pub incomplete_details: Option<IncompleteDetails>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub object: String,
    pub role: String,
    pub run_id: Option<String>,
    pub status: String,
    pub thread_id: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunMessageDelta {
    pub content: Option<Value>,
    pub role: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunMessageDeltaEvent {
    pub id: String,
    pub delta: RunMessageDelta,
    pub object: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStep {
    pub id: String,
    pub assistant_id: String,
    pub cancelled_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub created_at: i64,
    pub expired_at: Option<i64>,
    pub failed_at: Option<i64>,
    pub last_error: Option<LastError>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub object: String,
    pub run_id: String,
    pub status: String,
    pub step_details: Value,
    pub thread_id: String,
    pub r#type: String,
    pub usage: Option<RunUsage>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStepDelta {
    pub step_details: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStepDeltaEvent {
    pub id: String,
    pub delta: RunStepDelta,
    pub object: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
