use std::collections::BTreeMap;

use serde::Serialize;

use crate::completion_bridge::events::Event;
use crate::pydantic_ai_transformation::wire::Part;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Submitted,
    Working,
    Completed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct StatusState {
    pub state: TaskState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEventResult {
    pub context_id: String,
    pub history: Vec<HistoryMessage>,
    pub id: String,
    pub kind: &'static str,
    pub status: StatusState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMessage {
    pub context_id: String,
    pub kind: &'static str,
    pub message_id: String,
    pub parts: Vec<Part>,
    pub role: &'static str,
    pub task_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusUpdateResult {
    pub context_id: String,
    #[serde(rename = "final")]
    pub final_event: bool,
    pub kind: &'static str,
    pub status: StatusState,
    pub task_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactUpdateResult {
    pub context_id: String,
    pub kind: &'static str,
    pub task_id: String,
    pub artifact: ArtifactChunk,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactChunk {
    pub artifact_id: String,
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum StreamResult {
    Task(TaskEventResult),
    Status(StatusUpdateResult),
    Artifact(ArtifactUpdateResult),
}

pub type StreamEvent = Event<StreamResult>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamIds {
    pub request_id: String,
    pub task_id: String,
    pub context_id: String,
    pub artifact_id: String,
}

impl StreamIds {
    #[must_use]
    pub fn task_event(&self, input_message_id: String, input_parts: Vec<Part>) -> StreamEvent {
        Event::from((
            self.request_id.clone(),
            StreamResult::Task(TaskEventResult {
                context_id: self.context_id.clone(),
                history: vec![HistoryMessage {
                    context_id: self.context_id.clone(),
                    kind: "message",
                    message_id: input_message_id,
                    parts: input_parts,
                    role: "user",
                    task_id: self.task_id.clone(),
                }],
                id: self.task_id.clone(),
                kind: "task",
                status: StatusState {
                    state: TaskState::Submitted,
                },
            }),
        ))
    }

    #[must_use]
    pub fn status_event(&self, state: TaskState, final_event: bool) -> StreamEvent {
        Event::from((
            self.request_id.clone(),
            StreamResult::Status(StatusUpdateResult {
                context_id: self.context_id.clone(),
                final_event,
                kind: "status-update",
                status: StatusState { state },
                task_id: self.task_id.clone(),
            }),
        ))
    }

    #[must_use]
    pub fn artifact_event(&self, text: String) -> StreamEvent {
        Event::from((
            self.request_id.clone(),
            StreamResult::Artifact(ArtifactUpdateResult {
                context_id: self.context_id.clone(),
                kind: "artifact-update",
                task_id: self.task_id.clone(),
                artifact: ArtifactChunk {
                    artifact_id: self.artifact_id.clone(),
                    parts: vec![Part {
                        kind: Some("text".to_owned()),
                        text: Some(text),
                        extra: BTreeMap::new(),
                    }],
                },
            }),
        ))
    }
}
