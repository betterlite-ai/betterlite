use betterlite_core::agents::json::AgentJson;

use crate::client::convert::ConvertError;
use crate::client::convert::parts::parts_to_compat;
use crate::client::convert::to_compat_message;
use crate::client::convert::{copy_fields, named, push_optional};

pub fn task_to_compat(task: &AgentJson) -> Result<AgentJson, ConvertError> {
    let mut fields = task_fields(task)?;
    copy_fields(task, &mut fields, &["metadata"]);
    fields.extend(task_lists(task)?);
    Ok(AgentJson::Object(fields.into_iter().collect()))
}

fn task_fields(task: &AgentJson) -> Result<Vec<(String, AgentJson)>, ConvertError> {
    Ok(named(vec![
        ("kind", AgentJson::String("task".to_owned())),
        ("id", task.get("id").cloned().unwrap_or_default()),
        (
            "contextId",
            task.get("contextId").cloned().unwrap_or_default(),
        ),
        ("status", status_to_compat(task.get("status"))?),
    ]))
}

fn task_lists(task: &AgentJson) -> Result<Vec<(String, AgentJson)>, ConvertError> {
    let mut fields = Vec::new();
    let history = array_to_compat(task.get("history"), to_compat_message)?;
    push_optional(&mut fields, "history", history);
    let artifacts = array_to_compat(task.get("artifacts"), artifact_to_compat)?;
    push_optional(&mut fields, "artifacts", artifacts);
    Ok(fields)
}

pub fn status_update_to_compat(update: &AgentJson) -> Result<AgentJson, ConvertError> {
    let mut fields = event_fields("status-update", update);
    fields.extend(status_fields(update)?);
    copy_fields(update, &mut fields, &["metadata"]);
    Ok(AgentJson::Object(fields.into_iter().collect()))
}

pub fn artifact_update_to_compat(update: &AgentJson) -> Result<AgentJson, ConvertError> {
    let mut fields = event_fields("artifact-update", update);
    let artifact = artifact_to_compat(&update.get("artifact").cloned().unwrap_or_default())?;
    fields.push(("artifact".to_owned(), artifact));
    copy_fields(update, &mut fields, &["append", "lastChunk", "metadata"]);
    Ok(AgentJson::Object(fields.into_iter().collect()))
}

fn event_fields(name: &str, update: &AgentJson) -> Vec<(String, AgentJson)> {
    named(vec![
        ("kind", AgentJson::String(name.to_owned())),
        ("taskId", update.get("taskId").cloned().unwrap_or_default()),
        (
            "contextId",
            update.get("contextId").cloned().unwrap_or_default(),
        ),
    ])
}

fn status_fields(update: &AgentJson) -> Result<Vec<(String, AgentJson)>, ConvertError> {
    Ok(named(vec![
        ("status", status_to_compat(update.get("status"))?),
        ("final", AgentJson::Bool(is_final(core_state(update)))),
    ]))
}

fn core_state(update: &AgentJson) -> Option<&str> {
    update
        .get("status")
        .and_then(|status| status.get("state"))
        .and_then(AgentJson::as_str)
}

fn is_final(state: Option<&str>) -> bool {
    matches!(
        state,
        Some(
            "TASK_STATE_COMPLETED"
                | "TASK_STATE_CANCELED"
                | "TASK_STATE_FAILED"
                | "TASK_STATE_REJECTED"
        )
    )
}

fn status_to_compat(status: Option<&AgentJson>) -> Result<AgentJson, ConvertError> {
    let Some(present) = status else {
        return Ok(unknown_status());
    };
    let mut fields = named(vec![(
        "state",
        compat_state(present.get("state").and_then(AgentJson::as_str)),
    )]);
    copy_fields(present, &mut fields, &["timestamp"]);
    let message = present.get("message").map(to_compat_message).transpose()?;
    push_optional(&mut fields, "message", message);
    Ok(AgentJson::Object(fields.into_iter().collect()))
}

fn unknown_status() -> AgentJson {
    AgentJson::Object(
        named(vec![("state", AgentJson::String("unknown".to_owned()))])
            .into_iter()
            .collect(),
    )
}

fn compat_state(state: Option<&str>) -> AgentJson {
    let Some(name) = state else {
        return AgentJson::String("unknown".to_owned());
    };
    let lowered = name.strip_prefix("TASK_STATE_").unwrap_or_default();
    let compat = lowered.to_lowercase().replace('_', "-");
    let known = matches!(
        compat.as_str(),
        "submitted"
            | "working"
            | "completed"
            | "failed"
            | "canceled"
            | "input-required"
            | "rejected"
            | "auth-required"
    );
    AgentJson::String(if known { compat } else { "unknown".to_owned() })
}

fn artifact_to_compat(artifact: &AgentJson) -> Result<AgentJson, ConvertError> {
    let id = artifact.get("artifactId").cloned().unwrap_or_default();
    let parts = parts_to_compat(artifact.get("parts"))?;
    let mut fields = named(vec![("artifactId", id), ("parts", parts)]);
    copy_fields(
        artifact,
        &mut fields,
        &["name", "description", "metadata", "extensions"],
    );
    Ok(AgentJson::Object(fields.into_iter().collect()))
}

fn array_to_compat(
    values: Option<&AgentJson>,
    convert: fn(&AgentJson) -> Result<AgentJson, ConvertError>,
) -> Result<Option<AgentJson>, ConvertError> {
    let Some(AgentJson::Array(items)) = values else {
        return Ok(None);
    };
    let converted = items.iter().map(convert).collect::<Result<Vec<_>, _>>()?;
    Ok(Some(AgentJson::Array(converted)))
}
