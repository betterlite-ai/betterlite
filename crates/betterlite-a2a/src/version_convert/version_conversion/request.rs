use betterlite_core::agents::json::AgentJson;

use crate::version_convert::JsonObject;

use super::error::ConvertError;
use super::json::encode;

mod push_config;
mod records;

use records::{
    CancelTaskRequest, DeletePushConfigParams, DeletePushConfigRequest, GetPushConfigRequest,
    GetTaskRequest, ListPushConfigsRequest, ListTasksParams, ListTasksRequest, PushConfigParams,
    SubscribeToTaskRequest, TaskIdParams, TaskQueryParams, TaskState, integer_value,
};

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

pub(super) fn lower(request: &AgentJson, method: &str) -> Result<JsonObject, ConvertError> {
    match method {
        "tasks/get" => lower_get(request),
        "tasks/cancel" => lower_cancel(request),
        "tasks/resubscribe" => lower_resubscribe(request),
        "tasks/pushNotificationConfig/set" => push_config::lower_set(request),
        "tasks/pushNotificationConfig/get" => lower_get_push_config(request),
        "tasks/pushNotificationConfig/list" => lower_list_push_configs(request),
        "tasks/pushNotificationConfig/delete" => lower_delete_push_config(request),
        "tasks/list" => lower_list_tasks(request),
        _ => serde_json::from_value(request.clone()).map_err(ConvertError::Decode),
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

#[rustfmt::skip]
fn lower_get(request: &AgentJson) -> Result<JsonObject, ConvertError> {
    let query: GetTaskRequest = serde_json::from_value(request.clone()).map_err(ConvertError::Decode)?;
    let history_length = query.history_length.map(|value| integer_value(value, "historyLength")).transpose()?;
    let params = TaskQueryParams { id: query.id, history_length };
    encode(&params)
}

#[rustfmt::skip]
fn lower_cancel(request: &AgentJson) -> Result<JsonObject, ConvertError> {
    let cancel: CancelTaskRequest = serde_json::from_value(request.clone()).map_err(ConvertError::Decode)?;
    let params = TaskIdParams { id: cancel.id, metadata: cancel.metadata };
    encode(&params)
}

#[rustfmt::skip]
fn lower_resubscribe(request: &AgentJson) -> Result<JsonObject, ConvertError> {
    let subscribe: SubscribeToTaskRequest = serde_json::from_value(request.clone()).map_err(ConvertError::Decode)?;
    let params = TaskIdParams { id: subscribe.id, metadata: None };
    encode(&params)
}

#[rustfmt::skip]
fn lower_get_push_config(request: &AgentJson) -> Result<JsonObject, ConvertError> {
    let query: GetPushConfigRequest = serde_json::from_value(request.clone()).map_err(ConvertError::Decode)?;
    let params = PushConfigParams { id: query.task_id, push_notification_config_id: Some(query.id).filter(|text| !text.is_empty()) };
    encode(&params)
}

#[rustfmt::skip]
fn lower_list_push_configs(request: &AgentJson) -> Result<JsonObject, ConvertError> {
    let query: ListPushConfigsRequest = serde_json::from_value(request.clone()).map_err(ConvertError::Decode)?;
    let params = PushConfigParams { id: query.task_id, push_notification_config_id: None };
    encode(&params)
}

#[rustfmt::skip]
fn lower_delete_push_config(request: &AgentJson) -> Result<JsonObject, ConvertError> {
    let delete: DeletePushConfigRequest = serde_json::from_value(request.clone()).map_err(ConvertError::Decode)?;
    let params = DeletePushConfigParams { id: delete.task_id, push_notification_config_id: delete.id };
    encode(&params)
}

fn lower_list_tasks(request: &AgentJson) -> Result<JsonObject, ConvertError> {
    let query: ListTasksRequest =
        serde_json::from_value(request.clone()).map_err(ConvertError::Decode)?;
    let page_size = query
        .page_size
        .map(|value| integer_value(value, "pageSize"))
        .transpose()?;
    let history_length = query
        .history_length
        .map(|value| integer_value(value, "historyLength"))
        .transpose()?;
    let params = ListTasksParams {
        tenant: Some(query.tenant).filter(|text| !text.is_empty()),
        context_id: Some(query.context_id).filter(|text| !text.is_empty()),
        status: query.status.and_then(task_state),
        page_size,
        page_token: Some(query.page_token).filter(|text| !text.is_empty()),
        history_length,
        status_timestamp_after: query.status_timestamp_after.filter(|text| !text.is_empty()),
        include_artifacts: query.include_artifacts,
    };
    encode(&params)
}

fn task_state(value: TaskState) -> Option<String> {
    match value {
        TaskState::Name(name) => {
            if PROTO_TASK_STATES.contains(&name.as_str()) {
                return map_task_state(&name);
            }
            // protobuf reads an unnamed enum value with plain int(), which trims and rejects floats
            state_name(name.trim().parse::<i64>().ok()?)
        }
        TaskState::Number(number) => state_name(truncated_index(&number)?),
    }
}

fn truncated_index(number: &serde_json::Number) -> Option<i64> {
    if let Some(index) = number.as_i64() {
        return Some(index);
    }
    let value = number.as_f64()?;
    let whole = value.trunc();
    format!("{whole:.0}").parse::<i64>().ok()
}

fn state_name(index: i64) -> Option<String> {
    let position = usize::try_from(index).ok()?;
    let name = PROTO_TASK_STATES.get(position)?;
    map_task_state(name)
}
