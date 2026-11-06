use betterlite_core::agents::json::AgentJson;
use serde::{Deserialize, Serialize};

use super::MessageCommon;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreSendMessageRequest { pub message: CoreMessage, #[serde(default, skip_serializing_if = "Option::is_none")] pub configuration: Option<CoreSendMessageConfiguration>, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreSendMessageConfiguration { #[serde(default, skip_serializing_if = "Vec::is_empty")] pub accepted_output_modes: Vec<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub task_push_notification_config: Option<CorePushNotificationConfig>, #[serde(default, skip_serializing_if = "Option::is_none")] pub history_length: Option<i64>, #[serde(default)] pub return_immediately: bool }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CorePushNotificationConfig { pub url: String, #[serde(default, skip_serializing_if = "Option::is_none")] pub id: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub token: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub authentication: Option<CoreAuthenticationInfo> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreAuthenticationInfo { #[serde(default, skip_serializing_if = "Option::is_none")] pub scheme: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub credentials: Option<String> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[rustfmt::skip]
pub struct CoreMessage { #[serde(default)] pub role: CoreRole, pub parts: Vec<CorePart>, #[serde(flatten)] pub common: MessageCommon }

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[rustfmt::skip]
pub enum CoreRole { #[serde(rename = "ROLE_UNSPECIFIED")] #[default] Unspecified, #[serde(rename = "ROLE_USER")] User, #[serde(rename = "ROLE_AGENT")] Agent }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)] #[rustfmt::skip]
pub enum CorePart { Text(CoreTextPart), Data(CoreDataPart), Raw(CoreRawPart), Url(CoreUrlPart) }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreTextPart { pub text: String, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreDataPart { pub data: AgentJson, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreRawPart { pub raw: String, #[serde(default, skip_serializing_if = "Option::is_none")] pub media_type: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub filename: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreUrlPart { pub url: String, #[serde(default, skip_serializing_if = "Option::is_none")] pub media_type: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub filename: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreTask { pub id: String, #[serde(default)] pub context_id: Option<String>, #[serde(default)] pub status: Option<CoreTaskStatus>, #[serde(default)] pub history: Vec<CoreMessage>, #[serde(default)] pub artifacts: Vec<CoreArtifact>, #[serde(default)] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreTaskStatus { #[serde(default)] pub state: CoreTaskState, #[serde(default)] pub message: Option<CoreMessage>, #[serde(default)] pub timestamp: Option<String> }

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[rustfmt::skip]
pub enum CoreTaskState { #[serde(rename = "TASK_STATE_UNSPECIFIED")] #[default] Unspecified, #[serde(rename = "TASK_STATE_SUBMITTED")] Submitted, #[serde(rename = "TASK_STATE_WORKING")] Working, #[serde(rename = "TASK_STATE_COMPLETED")] Completed, #[serde(rename = "TASK_STATE_FAILED")] Failed, #[serde(rename = "TASK_STATE_CANCELED")] Canceled, #[serde(rename = "TASK_STATE_INPUT_REQUIRED")] InputRequired, #[serde(rename = "TASK_STATE_REJECTED")] Rejected, #[serde(rename = "TASK_STATE_AUTH_REQUIRED")] AuthRequired }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreArtifact { pub artifact_id: String, #[serde(default)] pub name: Option<String>, #[serde(default)] pub description: Option<String>, #[serde(default)] pub parts: Vec<CorePart>, #[serde(default)] pub metadata: Option<AgentJson>, #[serde(default)] pub extensions: Vec<String> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreStatusUpdate { pub task_id: String, #[serde(default)] pub context_id: Option<String>, #[serde(default)] pub status: Option<CoreTaskStatus>, #[serde(default)] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreArtifactUpdate { pub task_id: String, #[serde(default)] pub context_id: Option<String>, pub artifact: CoreArtifact, #[serde(default)] pub append: Option<bool>, #[serde(default)] pub last_chunk: Option<bool>, #[serde(default)] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreStreamEnvelope { #[serde(default)] pub message: Option<CoreMessage>, #[serde(default)] pub task: Option<CoreTask>, #[serde(default)] pub status_update: Option<CoreStatusUpdate>, #[serde(default)] pub artifact_update: Option<CoreArtifactUpdate> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct CoreSendResponse { #[serde(default)] pub message: Option<CoreMessage>, #[serde(default)] pub task: Option<CoreTask> }

#[derive(Clone, Debug, Eq, PartialEq)]
#[rustfmt::skip]
pub enum CoreStreamEvent { Message(CoreMessage), Task(CoreTask), StatusUpdate(CoreStatusUpdate), ArtifactUpdate(CoreArtifactUpdate) }
