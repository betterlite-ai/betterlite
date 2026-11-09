use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use serde::{Deserialize, Serialize};

pub type JsonMap = BTreeMap<String, AgentJson>;

// each field is read under its camelCase name or its snake_case name, and undeclared keys are ignored
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSendParamsV03 {
    pub message: MessageV03,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<SendConfigV03>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonMap>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageV03 {
    #[serde(default)]
    pub kind: MessageKindV03,
    #[serde(alias = "message_id")]
    pub message_id: String,
    pub role: RoleV03,
    pub parts: Vec<PartV03>,
    #[serde(default, alias = "context_id", skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    #[serde(default, alias = "task_id", skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(
        default,
        alias = "reference_task_ids",
        skip_serializing_if = "Option::is_none"
    )]
    pub reference_task_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonMap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKindV03 {
    #[default]
    Message,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RoleV03 {
    Agent,
    User,
}

// the SDK's smart union picks FilePart when a part carries a file next to another content key
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum PartV03 {
    File {
        #[serde(default)]
        kind: FileKindV03,
        file: FileV03,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<JsonMap>,
    },
    Text {
        #[serde(default)]
        kind: TextKindV03,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<JsonMap>,
    },
    Data {
        #[serde(default)]
        kind: DataKindV03,
        data: JsonMap,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<JsonMap>,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKindV03 {
    #[default]
    File,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TextKindV03 {
    #[default]
    Text,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DataKindV03 {
    #[default]
    Data,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum FileV03 {
    Bytes {
        bytes: String,
        #[serde(
            default,
            rename = "mimeType",
            alias = "mime_type",
            skip_serializing_if = "Option::is_none"
        )]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Uri {
        uri: String,
        #[serde(
            default,
            rename = "mimeType",
            alias = "mime_type",
            skip_serializing_if = "Option::is_none"
        )]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendConfigV03 {
    #[serde(
        default,
        alias = "accepted_output_modes",
        skip_serializing_if = "Option::is_none"
    )]
    pub accepted_output_modes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking: Option<bool>,
    #[serde(
        default,
        alias = "history_length",
        skip_serializing_if = "Option::is_none"
    )]
    pub history_length: Option<i64>,
    #[serde(
        default,
        alias = "push_notification_config",
        skip_serializing_if = "Option::is_none"
    )]
    pub push_notification_config: Option<PushConfigV03>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushConfigV03 {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication: Option<PushAuthV03>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushAuthV03 {
    pub schemes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<String>,
}
