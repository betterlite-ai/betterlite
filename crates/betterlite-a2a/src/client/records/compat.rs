use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::JsonRpcId;
use serde::Deserialize;

use super::MessageCommon;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct SendMessageRequest {
    pub id: JsonRpcId,
    pub params: MessageSendParams,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MessageSendParams {
    pub message: Message,
    pub configuration: Option<MessageSendConfiguration>,
    pub metadata: Option<AgentJson>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MessageSendConfiguration {
    pub accepted_output_modes: Option<Vec<String>>,
    pub blocking: Option<bool>,
    pub history_length: Option<i64>,
    pub push_notification_config: Option<PushNotificationConfig>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PushNotificationConfig {
    pub url: String,
    pub id: Option<String>,
    pub token: Option<String>,
    pub authentication: Option<AuthenticationInfo>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationInfo {
    #[serde(default)]
    pub schemes: Vec<String>,
    pub credentials: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<AgentJson>,
    #[serde(flatten)]
    pub common: MessageCommon,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Agent,
    User,
}
