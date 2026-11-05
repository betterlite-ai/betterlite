use serde::Serialize;

use super::wire::TextPart;
use betterlite_core::agents::json::AgentJson;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEventResult {
    pub context_id: String,
    pub history: Vec<HistoryMessage>,
    pub id: String,
    pub kind: String,
    pub status: SubmittedStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMessage {
    pub context_id: String,
    pub kind: String,
    pub message_id: String,
    pub parts: Vec<AgentJson>,
    pub role: String,
    pub task_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SubmittedStatus {
    pub state: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusUpdateResult {
    pub context_id: String,
    #[serde(rename = "final")]
    pub final_event: bool,
    pub kind: String,
    pub status: StatusRecord,
    pub task_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StatusRecord {
    pub state: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<StatusMessage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusMessage {
    pub context_id: String,
    pub kind: String,
    pub message_id: String,
    pub parts: Vec<TextPart>,
    pub role: String,
    pub task_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactUpdateResult {
    pub artifact: Artifact,
    pub context_id: String,
    pub kind: String,
    pub task_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub artifact_id: String,
    pub name: String,
    pub parts: Vec<TextPart>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum StreamResult {
    Task(TaskEventResult),
    Status(StatusUpdateResult),
    Artifact(ArtifactUpdateResult),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Event<T> {
    pub id: String,
    pub jsonrpc: String,
    pub result: T,
}

impl<T> From<(String, T)> for Event<T> {
    fn from((id, result): (String, T)) -> Self {
        Self {
            id,
            jsonrpc: "2.0".to_owned(),
            result,
        }
    }
}

pub type StreamEvent = Event<StreamResult>;
