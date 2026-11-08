use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::JsonRpcId;

use crate::client::convert::messages::to_compat_message;
use crate::client::convert::response::to_compat_task;
use crate::client::convert::to_compat_stream_response;
use crate::client::records::core::{
    CoreSendResponse, CoreStreamEnvelope, CoreStreamEvent, CoreTask,
};
use crate::version_convert::JsonObject;

use super::error::ConvertError;
use super::json::encode;

pub(super) fn send_result(value: &AgentJson) -> Result<JsonObject, ConvertError> {
    let response: CoreSendResponse =
        serde_json::from_value(value.clone()).map_err(ConvertError::Decode)?;
    if let Some(task) = response.task {
        let converted = to_compat_task(&task);
        return encode(&converted);
    }
    if let Some(message) = response.message {
        return encode(&to_compat_message(&message));
    }
    Err(ConvertError::MissingEvent {
        label: "send result",
    })
}

pub(super) fn stream_result(
    value: &AgentJson,
    request_id: Option<&JsonRpcId>,
) -> Result<JsonObject, ConvertError> {
    let envelope: CoreStreamEnvelope =
        serde_json::from_value(value.clone()).map_err(ConvertError::Decode)?;
    let event = stream_event(envelope).ok_or(ConvertError::MissingEvent {
        label: "stream result",
    })?;
    let id = request_id
        .cloned()
        .unwrap_or_else(|| JsonRpcId::String(String::new()));
    let response = to_compat_stream_response(event, id);
    encode(&response.result)
}

pub(super) fn task(value: &AgentJson) -> Result<JsonObject, ConvertError> {
    let task: CoreTask = serde_json::from_value(value.clone()).map_err(ConvertError::Decode)?;
    let converted = to_compat_task(&task);
    encode(&converted)
}

pub(super) fn list_tasks(value: &AgentJson) -> Result<JsonObject, ConvertError> {
    let Some(AgentJson::Array(items)) = value.get("tasks") else {
        return Err(ConvertError::NotAnObject);
    };
    let mut converted = Vec::with_capacity(items.len());
    for item in items {
        if item.is_object() {
            let converted_item = task(item)?;
            converted.push(AgentJson::Object(converted_item.into_iter().collect()));
        } else {
            converted.push(item.clone());
        }
    }
    let AgentJson::Object(mut out) = value.clone() else {
        return Err(ConvertError::NotAnObject);
    };
    out.insert("tasks".to_owned(), AgentJson::Array(converted));
    Ok(out.into_iter().collect())
}

fn stream_event(envelope: CoreStreamEnvelope) -> Option<CoreStreamEvent> {
    if let Some(message) = envelope.message {
        return Some(CoreStreamEvent::Message(message));
    }
    if let Some(task) = envelope.task {
        return Some(CoreStreamEvent::Task(task));
    }
    if let Some(update) = envelope.status_update {
        return Some(CoreStreamEvent::StatusUpdate(update));
    }
    envelope
        .artifact_update
        .map(CoreStreamEvent::ArtifactUpdate)
}
