use betterlite_core::agents::json::AgentJson;

use crate::version_convert::JsonObject;

use super::error::ConvertError;
use super::fields::{Copy, boolean, copy, insert, integer};

mod push_config;

const V0_3_TASK_STATES: [&str; 9] = [
    "unknown",
    "submitted",
    "working",
    "completed",
    "failed",
    "canceled",
    "input-required",
    "rejected",
    "auth-required",
];

const PROTO_TASK_STATES: [&str; 9] = [
    "TASK_STATE_UNSPECIFIED",
    "TASK_STATE_SUBMITTED",
    "TASK_STATE_WORKING",
    "TASK_STATE_COMPLETED",
    "TASK_STATE_FAILED",
    "TASK_STATE_CANCELED",
    "TASK_STATE_INPUT_REQUIRED",
    "TASK_STATE_REJECTED",
    "TASK_STATE_AUTH_REQUIRED",
];

pub(super) fn lower(params: &JsonObject, method: &str) -> Result<JsonObject, ConvertError> {
    match method {
        "tasks/get" => lower_get(params),
        "tasks/cancel" => lower_cancel(params),
        "tasks/resubscribe" => lower_resubscribe(params),
        "tasks/pushNotificationConfig/set" => push_config::lower_set(params),
        "tasks/pushNotificationConfig/get" => push_config::lower_get(params),
        "tasks/pushNotificationConfig/list" => push_config::lower_list(params),
        "tasks/pushNotificationConfig/delete" => push_config::lower_delete(params),
        "tasks/list" => lower_list_tasks(params),
        _ => Ok(params.clone()),
    }
}

pub(super) fn map_task_state(name: &str) -> Option<String> {
    let base = name.strip_prefix("TASK_STATE_").unwrap_or(name);
    if base == "UNSPECIFIED" {
        return None;
    }
    let candidate = base.to_ascii_lowercase().replace('_', "-");
    V0_3_TASK_STATES
        .contains(&candidate.as_str())
        .then_some(candidate)
}

fn lower_get(params: &JsonObject) -> Result<JsonObject, ConvertError> {
    let mut out = JsonObject::new();
    copy(Copy::TextOrDefault, &mut out, "id", params, "id")?;
    insert(
        &mut out,
        "historyLength",
        integer(params, "historyLength")?.map(AgentJson::from),
    );
    Ok(out)
}

fn lower_cancel(params: &JsonObject) -> Result<JsonObject, ConvertError> {
    let mut out = JsonObject::new();
    copy(Copy::TextOrDefault, &mut out, "id", params, "id")?;
    copy(Copy::Object, &mut out, "metadata", params, "metadata")?;
    Ok(out)
}

fn lower_resubscribe(params: &JsonObject) -> Result<JsonObject, ConvertError> {
    let mut out = JsonObject::new();
    copy(Copy::TextOrDefault, &mut out, "id", params, "id")?;
    Ok(out)
}

fn lower_list_tasks(params: &JsonObject) -> Result<JsonObject, ConvertError> {
    let mut out = JsonObject::new();
    copy(Copy::NonEmptyText, &mut out, "tenant", params, "tenant")?;
    copy(
        Copy::NonEmptyText,
        &mut out,
        "contextId",
        params,
        "contextId",
    )?;
    insert(&mut out, "status", task_state_field(params)?);
    copy(
        Copy::NonEmptyText,
        &mut out,
        "pageToken",
        params,
        "pageToken",
    )?;
    copy(
        Copy::NonEmptyText,
        &mut out,
        "statusTimestampAfter",
        params,
        "statusTimestampAfter",
    )?;
    copy_list_limits(&mut out, params)?;
    Ok(out)
}

fn copy_list_limits(out: &mut JsonObject, params: &JsonObject) -> Result<(), ConvertError> {
    insert(
        out,
        "pageSize",
        integer(params, "pageSize")?
            .filter(|number| *number != 0)
            .map(AgentJson::from),
    );
    insert(
        out,
        "historyLength",
        integer(params, "historyLength")?.map(AgentJson::from),
    );
    insert(
        out,
        "includeArtifacts",
        boolean(params, "includeArtifacts")?.and_then(|flag| flag.then_some(AgentJson::Bool(true))),
    );
    Ok(())
}

fn task_state_field(params: &JsonObject) -> Result<Option<AgentJson>, ConvertError> {
    match params.get("status") {
        None | Some(AgentJson::Null) => Ok(None),
        Some(AgentJson::String(name)) => Ok(map_task_state(name).map(AgentJson::String)),
        Some(AgentJson::Number(number)) => {
            let name = number.as_i64().and_then(state_name);
            Ok(name.and_then(map_task_state).map(AgentJson::String))
        }
        Some(_) => Err(ConvertError::Field {
            field: "status".to_owned(),
            expected: "a task state name or number",
        }),
    }
}

fn state_name(index: i64) -> Option<&'static str> {
    let position = usize::try_from(index).ok()?;
    let name = PROTO_TASK_STATES.get(position)?;
    Some(*name)
}
