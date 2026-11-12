use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::ExposedSecret;
use crate::sections::guardrail_identity::item::GuardrailValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LakeraOnFlagged {
    Block,
    Monitor,
    InjectSystemMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LakeraV2GuardrailConfigModel {
    #[serde(
        default,
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default = "default_payload")]
    pub payload: Option<bool>,
    #[serde(default = "default_breakdown")]
    pub breakdown: Option<bool>,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(default = "default_dev_info")]
    pub dev_info: Option<bool>,
    #[serde(default = "default_on_flagged")]
    pub on_flagged: Option<LakeraOnFlagged>,
    #[serde(default)]
    pub advisory_system_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LassoGuardrailConfigModel {
    #[serde(default)]
    pub lasso_user_id: Option<String>,
    #[serde(default)]
    pub lasso_conversation_id: Option<String>,
    #[serde(default = "default_mask")]
    pub mask: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeepKeepGuardrailConfigModel {
    pub deepkeep_firewall_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PillarGuardrailConfigModel {
    #[serde(default = "default_on_flagged_action")]
    pub on_flagged_action: Option<String>,
    #[serde(default)]
    pub async_mode: Option<bool>,
    #[serde(default)]
    pub persist_session: Option<bool>,
    #[serde(default = "default_include_scanners")]
    pub include_scanners: Option<bool>,
    #[serde(default = "default_include_evidence")]
    pub include_evidence: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NomaGuardrailConfigModel {
    #[serde(default = "default_use_v2")]
    pub use_v2: Option<bool>,
    #[serde(default)]
    pub application_id: Option<String>,
    #[serde(default)]
    pub gateway_name: Option<String>,
    #[serde(default)]
    pub monitor_mode: Option<bool>,
    #[serde(default)]
    pub block_failures: Option<bool>,
    #[serde(default)]
    pub anonymize_input: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZscalerAIGuardConfigModel {
    #[serde(default)]
    pub policy_id: Option<i64>,
    #[serde(default = "default_send_user_alias")]
    pub send_user_api_key_alias: Option<bool>,
    #[serde(default = "default_send_user_id")]
    pub send_user_api_key_user_id: Option<bool>,
    #[serde(default = "default_send_team_id")]
    pub send_user_api_key_team_id: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JavelinGuardrailConfigModel {
    pub guard_name: Option<String>,
    pub api_version: Option<String>,
    pub metadata: Option<BTreeMap<String, GuardrailValue>>,
    pub application: Option<String>,
    pub config: Option<BTreeMap<String, GuardrailValue>>,
}

// LiteLLM declares these `bool | None` with a non-None default, so an absent key takes the value and null stays None
#[must_use]
pub const fn default_payload() -> Option<bool> {
    Some(true)
}

#[must_use]
pub const fn default_breakdown() -> Option<bool> {
    Some(true)
}

#[must_use]
pub const fn default_dev_info() -> Option<bool> {
    Some(true)
}

#[must_use]
pub const fn default_on_flagged() -> Option<LakeraOnFlagged> {
    Some(LakeraOnFlagged::Block)
}

#[must_use]
pub const fn default_mask() -> Option<bool> {
    Some(false)
}

#[must_use]
pub fn default_on_flagged_action() -> Option<String> {
    Some("monitor".to_owned())
}

#[must_use]
pub const fn default_include_scanners() -> Option<bool> {
    Some(true)
}

#[must_use]
pub const fn default_include_evidence() -> Option<bool> {
    Some(true)
}

#[must_use]
pub const fn default_use_v2() -> Option<bool> {
    Some(false)
}

#[must_use]
pub const fn default_send_user_alias() -> Option<bool> {
    Some(false)
}

#[must_use]
pub const fn default_send_user_id() -> Option<bool> {
    Some(false)
}

#[must_use]
pub const fn default_send_team_id() -> Option<bool> {
    Some(false)
}
