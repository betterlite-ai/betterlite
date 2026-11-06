use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::JsonRpcId;
use serde::{Deserialize, Serialize};

use super::MessageCommon;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[rustfmt::skip]
pub struct SendMessageRequest { pub id: JsonRpcId, pub params: MessageSendParams }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct MessageSendParams { pub message: Message, #[serde(default, skip_serializing_if = "Option::is_none")] pub configuration: Option<MessageSendConfiguration>, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct MessageSendConfiguration { #[serde(default, skip_serializing_if = "Option::is_none")] pub accepted_output_modes: Option<Vec<String>>, #[serde(default, skip_serializing_if = "Option::is_none")] pub blocking: Option<bool>, #[serde(default, skip_serializing_if = "Option::is_none")] pub history_length: Option<i64>, #[serde(default, skip_serializing_if = "Option::is_none")] pub push_notification_config: Option<PushNotificationConfig> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct PushNotificationConfig { pub url: String, #[serde(default, skip_serializing_if = "Option::is_none")] pub id: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub token: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub authentication: Option<AuthenticationInfo> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct AuthenticationInfo { #[serde(default)] pub schemes: Vec<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub credentials: Option<String> }

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")] #[rustfmt::skip]
pub enum MessageKind { #[default] Message }

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")] #[rustfmt::skip]
pub enum TaskKind { #[default] Task }

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")] #[rustfmt::skip]
pub enum StatusUpdateKind { #[default] StatusUpdate }

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")] #[rustfmt::skip]
pub enum ArtifactUpdateKind { #[default] ArtifactUpdate }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[rustfmt::skip]
pub struct Message { #[serde(default)] pub kind: MessageKind, pub role: Role, pub parts: Vec<Part>, #[serde(flatten)] pub common: MessageCommon }

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")] #[rustfmt::skip]
pub enum Role { Agent, User }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")] #[rustfmt::skip]
pub enum Part { Text(TextPart), Data(DataPart), File(FilePart) }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct TextPart { pub text: String, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct DataPart { pub data: AgentJson, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct FilePart { pub file: FileContent, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)] #[rustfmt::skip]
pub enum FileContent { Bytes(FileWithBytes), Uri(FileWithUri) }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct FileWithBytes { pub bytes: String, #[serde(default, skip_serializing_if = "Option::is_none")] pub mime_type: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub name: Option<String> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct FileWithUri { pub uri: String, #[serde(default, skip_serializing_if = "Option::is_none")] pub mime_type: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub name: Option<String> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct Task { #[serde(default)] pub kind: TaskKind, pub id: String, pub context_id: String, pub status: TaskStatus, #[serde(default, skip_serializing_if = "Option::is_none")] pub history: Option<Vec<Message>>, #[serde(default, skip_serializing_if = "Option::is_none")] pub artifacts: Option<Vec<Artifact>>, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct TaskStatus { pub state: TaskState, #[serde(default, skip_serializing_if = "Option::is_none")] pub message: Option<Message>, #[serde(default, skip_serializing_if = "Option::is_none")] pub timestamp: Option<String> }

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")] #[rustfmt::skip]
pub enum TaskState { Submitted, Working, InputRequired, Completed, Canceled, Failed, Rejected, AuthRequired, Unknown }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct Artifact { pub artifact_id: String, #[serde(default, skip_serializing_if = "Option::is_none")] pub name: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub description: Option<String>, pub parts: Vec<Part>, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson>, #[serde(default, skip_serializing_if = "Option::is_none")] pub extensions: Option<Vec<String>> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct TaskStatusUpdate { #[serde(default)] pub kind: StatusUpdateKind, pub task_id: String, pub context_id: String, pub status: TaskStatus, #[serde(rename = "final")] pub is_final: bool, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson> }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct TaskArtifactUpdate { #[serde(default)] pub kind: ArtifactUpdateKind, pub task_id: String, pub context_id: String, pub artifact: Artifact, #[serde(default, skip_serializing_if = "Option::is_none")] pub append: Option<bool>, #[serde(default, skip_serializing_if = "Option::is_none")] pub last_chunk: Option<bool>, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson> }
