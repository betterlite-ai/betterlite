use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct StraikerWebhookContent {
    #[serde(default)]
    pub texts: Vec<String>,
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_messages: Option<Vec<Object>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Object>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<Object>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct StraikerWebhookContext {
    pub call_surface: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_version: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct StraikerWebhookIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_team: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_user_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_user_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct StraikerWebhookUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
}

mod tests;
