use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

pub const ANALYZE_PATH: &str = "/v1/moderation/analyze/";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamoAiRequest {
    pub messages: Vec<GuardrailValue>,
    #[serde(rename = "policyIds", skip_serializing_if = "Option::is_none")]
    pub policy_ids: Option<Vec<String>>,
    #[serde(rename = "modelId", skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum FinalAction {
    #[default]
    None,
    Block,
    Warn,
    Redact,
    Sanitize,
}

impl FinalAction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Block => "BLOCK",
            Self::Warn => "WARN",
            Self::Redact => "REDACT",
            Self::Sanitize => "SANITIZE",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct DynamoAiResponse {
    #[serde(rename = "finalAction")]
    pub final_action: FinalAction,
    #[serde(rename = "appliedPolicies")]
    pub applied_policies: Vec<AppliedPolicy>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct AppliedPolicy {
    pub policy: PolicyInfo,
    pub outputs: Option<PolicyOutputs>,
    pub action: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PolicyInfo {
    pub id: Option<String>,
    pub name: Option<String>,
    pub method: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PolicyOutputs {
    pub action: Option<FinalAction>,
    pub message: Option<String>,
}
