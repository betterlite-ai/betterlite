use betterlite_core::agents::json::AgentJson;
use serde::Serialize;

use crate::client::convert::messages::to_core_message;
use crate::client::convert::parts::to_core_part;
use crate::client::records::compat;
use crate::client::records::core::{CoreMessage, CorePart};
use crate::version_convert::{JsonObject, to_object};

use super::error::ConvertError;
use super::json::{decode, encode};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
struct Task { id: String, #[serde(skip_serializing_if = "Option::is_none")] context_id: Option<String>, #[serde(skip_serializing_if = "Option::is_none")] status: Option<TaskStatus>, #[serde(skip_serializing_if = "Vec::is_empty")] history: Vec<CoreMessage>, #[serde(skip_serializing_if = "Vec::is_empty")] artifacts: Vec<Artifact>, #[serde(skip_serializing_if = "Option::is_none")] metadata: Option<AgentJson> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
struct TaskStatus { #[serde(skip_serializing_if = "Option::is_none")] state: Option<&'static str>, #[serde(skip_serializing_if = "Option::is_none")] message: Option<CoreMessage>, #[serde(skip_serializing_if = "Option::is_none")] timestamp: Option<String> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
struct Artifact { #[serde(rename = "artifactId")] id: String, #[serde(skip_serializing_if = "Option::is_none")] name: Option<String>, #[serde(skip_serializing_if = "Option::is_none")] description: Option<String>, parts: Vec<CorePart>, #[serde(skip_serializing_if = "Option::is_none")] metadata: Option<AgentJson>, #[serde(skip_serializing_if = "Vec::is_empty")] extensions: Vec<String> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
struct StatusUpdate { task_id: String, #[serde(skip_serializing_if = "Option::is_none")] context_id: Option<String>, status: TaskStatus, #[serde(skip_serializing_if = "Option::is_none")] metadata: Option<AgentJson> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
struct ArtifactUpdate { task_id: String, #[serde(skip_serializing_if = "Option::is_none")] context_id: Option<String>, artifact: Artifact, #[serde(skip_serializing_if = "Option::is_none")] append: Option<bool>, #[serde(skip_serializing_if = "Option::is_none")] last_chunk: Option<bool>, #[serde(skip_serializing_if = "Option::is_none")] metadata: Option<AgentJson> }

pub(super) fn send_result(value: &JsonObject) -> Result<JsonObject, ConvertError> {
    if value.get("kind").and_then(AgentJson::as_str) == Some("task") {
        return Ok(envelope("task", task_json(value)?));
    }
    Ok(envelope("message", message_json(value)?))
}

pub(super) fn stream_result(value: &JsonObject) -> Result<JsonObject, ConvertError> {
    match value.get("kind").and_then(AgentJson::as_str) {
        Some("task") => Ok(envelope("task", task_json(value)?)),
        Some("status-update") => Ok(envelope("statusUpdate", status_update_json(value)?)),
        Some("artifact-update") => Ok(envelope("artifactUpdate", artifact_update_json(value)?)),
        _ => Ok(envelope("message", message_json(value)?)),
    }
}

pub(super) fn task_json(value: &JsonObject) -> Result<JsonObject, ConvertError> {
    let task: compat::Task = decode(value)?;
    let converted = task_to_v1(task)?;
    encode(&converted)
}

pub(super) fn list_tasks(value: &JsonObject) -> Result<JsonObject, ConvertError> {
    let Some(AgentJson::Array(items)) = value.get("tasks") else {
        return Ok(value.clone());
    };
    let mut converted = Vec::with_capacity(items.len());
    for item in items {
        if item.is_object() {
            let fields = to_object(item).ok_or(ConvertError::NotAnObject)?;
            let task = task_json(&fields)?;
            converted.push(AgentJson::Object(task.into_iter().collect()));
        } else {
            converted.push(item.clone());
        }
    }
    let mut out = value.clone();
    out.insert("tasks".to_owned(), AgentJson::Array(converted));
    Ok(out)
}

fn envelope(key: &str, value: JsonObject) -> JsonObject {
    let mut envelope = JsonObject::new();
    envelope.insert(
        key.to_owned(),
        AgentJson::Object(value.into_iter().collect()),
    );
    envelope
}

fn message_json(value: &JsonObject) -> Result<JsonObject, ConvertError> {
    let message: compat::Message = decode(value)?;
    let converted = to_core_message(message)?;
    encode(&converted)
}

fn status_update_json(value: &JsonObject) -> Result<JsonObject, ConvertError> {
    let update: compat::TaskStatusUpdate = decode(value)?;
    let converted = status_update_to_v1(&update)?;
    encode(&converted)
}

fn artifact_update_json(value: &JsonObject) -> Result<JsonObject, ConvertError> {
    let update: compat::TaskArtifactUpdate = decode(value)?;
    let converted = artifact_update_to_v1(&update)?;
    encode(&converted)
}

fn task_to_v1(task: compat::Task) -> Result<Task, ConvertError> {
    let status = status_to_v1(&task.status)?;
    let mut history = Vec::new();
    if let Some(messages) = task.history {
        history.reserve(messages.len());
        for message in messages {
            history.push(to_core_message(message)?);
        }
    }
    let mut artifacts = Vec::new();
    if let Some(items) = task.artifacts {
        artifacts.reserve(items.len());
        for item in items {
            artifacts.push(artifact_to_v1(item)?);
        }
    }
    Ok(Task {
        id: task.id,
        context_id: Some(task.context_id).filter(|text| !text.is_empty()),
        status: Some(status),
        history,
        artifacts,
        metadata: task.metadata,
    })
}

fn status_to_v1(status: &compat::TaskStatus) -> Result<TaskStatus, ConvertError> {
    let message = match &status.message {
        Some(message) => Some(to_core_message(message.clone())?),
        None => None,
    };
    Ok(TaskStatus {
        state: core_state(status.state),
        message,
        timestamp: status.timestamp.clone().filter(|text| !text.is_empty()),
    })
}

fn artifact_to_v1(artifact: compat::Artifact) -> Result<Artifact, ConvertError> {
    let mut parts = Vec::with_capacity(artifact.parts.len());
    for part in artifact.parts {
        parts.push(to_core_part(part)?);
    }
    Ok(Artifact {
        id: artifact.artifact_id,
        name: artifact.name.filter(|text| !text.is_empty()),
        description: artifact.description.filter(|text| !text.is_empty()),
        parts,
        metadata: artifact.metadata,
        extensions: artifact.extensions.unwrap_or_default(),
    })
}

#[rustfmt::skip]
fn status_update_to_v1(update: &compat::TaskStatusUpdate) -> Result<StatusUpdate, ConvertError> {
    let status = status_to_v1(&update.status)?;
    let context_id = Some(update.context_id.clone()).filter(|text| !text.is_empty());
    Ok(StatusUpdate { task_id: update.task_id.clone(), context_id, status, metadata: update.metadata.clone() })
}

#[rustfmt::skip]
fn artifact_update_to_v1(update: &compat::TaskArtifactUpdate) -> Result<ArtifactUpdate, ConvertError> {
    let artifact = artifact_to_v1(update.artifact.clone())?;
    let context_id = Some(update.context_id.clone()).filter(|text| !text.is_empty());
    Ok(ArtifactUpdate { task_id: update.task_id.clone(), context_id, artifact, append: update.append, last_chunk: update.last_chunk, metadata: update.metadata.clone() })
}

const fn core_state(state: compat::TaskState) -> Option<&'static str> {
    match state {
        compat::TaskState::Unknown => None,
        compat::TaskState::Submitted => Some("TASK_STATE_SUBMITTED"),
        compat::TaskState::Working => Some("TASK_STATE_WORKING"),
        compat::TaskState::Completed => Some("TASK_STATE_COMPLETED"),
        compat::TaskState::Failed => Some("TASK_STATE_FAILED"),
        compat::TaskState::Canceled => Some("TASK_STATE_CANCELED"),
        compat::TaskState::InputRequired => Some("TASK_STATE_INPUT_REQUIRED"),
        compat::TaskState::Rejected => Some("TASK_STATE_REJECTED"),
        compat::TaskState::AuthRequired => Some("TASK_STATE_AUTH_REQUIRED"),
    }
}
