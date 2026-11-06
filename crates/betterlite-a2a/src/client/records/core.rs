use betterlite_core::agents::json::AgentJson;
use serde::{Deserialize, Serialize};

use super::MessageCommon;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreSendMessageRequest {
    pub message: CoreMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<AgentJson>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<AgentJson>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CoreMessage {
    #[serde(default)]
    pub role: CoreRole,
    pub parts: Vec<AgentJson>,
    #[serde(flatten)]
    pub common: MessageCommon,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum CoreRole {
    #[serde(rename = "ROLE_UNSPECIFIED")]
    #[default]
    Unspecified,
    #[serde(rename = "ROLE_USER")]
    User,
    #[serde(rename = "ROLE_AGENT")]
    Agent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoreStreamEvent {
    Message(AgentJson),
    Task(AgentJson),
    StatusUpdate(AgentJson),
    ArtifactUpdate(AgentJson),
}
