use betterlite_core::agents::json::AgentJson;
use serde::{Deserialize, Serialize};

use crate::version_convert::JsonObject;
use crate::version_convert::version_conversion::error::ConvertError;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct GetTaskRequest {
    pub id: String,
    pub history_length: Option<IntegerRepr>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct CancelTaskRequest {
    pub id: String,
    pub metadata: Option<JsonObject>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct SubscribeToTaskRequest {
    pub id: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct GetPushConfigRequest {
    pub task_id: String,
    pub id: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct ListPushConfigsRequest {
    pub task_id: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct DeletePushConfigRequest {
    pub task_id: String,
    pub id: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct ListTasksRequest {
    pub tenant: String,
    pub context_id: String,
    pub status: Option<TaskState>,
    pub page_size: Option<IntegerRepr>,
    pub page_token: String,
    pub history_length: Option<IntegerRepr>,
    pub status_timestamp_after: Option<String>,
    pub include_artifacts: Option<bool>,
}

// ProtoJSON accepts an integer as a number or as a string, including integral floats
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum IntegerRepr {
    Number(serde_json::Number),
    Text(String),
}

pub(super) fn integer_value(value: IntegerRepr, field: &'static str) -> Result<i64, ConvertError> {
    let text = match value {
        IntegerRepr::Number(number) => number.to_string(),
        IntegerRepr::Text(text) => text,
    };
    parse_integer(&text).ok_or(ConvertError::Integer { field })
}

fn parse_integer(text: &str) -> Option<i64> {
    if text.contains(' ') {
        return None;
    }
    if let Ok(number) = text.parse::<i64>() {
        return Some(number);
    }
    let value: f64 = text.parse().ok()?;
    if value.fract().abs() > f64::EPSILON {
        return None;
    }
    format!("{value:.0}").parse::<i64>().ok()
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum TaskState {
    Name(String),
    Number(serde_json::Number),
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct PushConfigEnvelope {
    pub config: Option<AgentJson>,
    pub push_notification_config: Option<AgentJson>,
    pub parent: Option<String>,
    pub config_id: Option<String>,
    pub task_id: Option<String>,
    pub url: Option<String>,
    pub id: Option<String>,
    pub token: Option<String>,
    pub authentication: Option<AuthenticationRequest>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct PushConfigRequest {
    pub id: Option<String>,
    pub task_id: Option<String>,
    pub url: Option<String>,
    pub token: Option<String>,
    pub authentication: Option<AuthenticationRequest>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct AuthenticationRequest {
    pub scheme: Option<String>,
    pub credentials: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TaskQueryParams {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_length: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TaskIdParams {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonObject>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushConfigParams {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_notification_config_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DeletePushConfigParams {
    pub id: String,
    pub push_notification_config_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TaskPushNotificationConfig {
    pub task_id: String,
    pub push_notification_config: PushNotificationConfig,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushNotificationConfig {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<PushConfigAuthentication>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushConfigAuthentication {
    pub schemes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListTasksParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_size: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_length: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_timestamp_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_artifacts: Option<bool>,
}
