use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_inline_default::serde_inline_default;

use super::ExposedSecret;
use crate::sections::guardrail_identity::item::GuardrailValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LakeraOnFlagged {
    Block,
    Monitor,
    InjectSystemMessage,
}

#[serde_inline_default]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LakeraV2GuardrailConfigModel {
    #[serde(
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub project_id: Option<String>,
    #[serde_inline_default(Some(true))]
    pub payload: Option<bool>,
    #[serde_inline_default(Some(true))]
    pub breakdown: Option<bool>,
    pub metadata: Option<BTreeMap<String, GuardrailValue>>,
    #[serde_inline_default(Some(true))]
    pub dev_info: Option<bool>,
    #[serde_inline_default(Some(LakeraOnFlagged::Block))]
    pub on_flagged: Option<LakeraOnFlagged>,
    pub advisory_system_message: Option<String>,
}

#[serde_inline_default]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LassoGuardrailConfigModel {
    pub lasso_user_id: Option<String>,
    pub lasso_conversation_id: Option<String>,
    #[serde_inline_default(Some(false))]
    pub mask: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeepKeepGuardrailConfigModel {
    pub deepkeep_firewall_id: Option<String>,
}

#[serde_inline_default]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PillarGuardrailConfigModel {
    #[serde_inline_default(Some("monitor".to_owned()))]
    pub on_flagged_action: Option<String>,
    pub async_mode: Option<bool>,
    pub persist_session: Option<bool>,
    #[serde_inline_default(Some(true))]
    pub include_scanners: Option<bool>,
    #[serde_inline_default(Some(true))]
    pub include_evidence: Option<bool>,
}

#[serde_inline_default]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NomaGuardrailConfigModel {
    #[serde_inline_default(Some(false))]
    pub use_v2: Option<bool>,
    pub application_id: Option<String>,
    pub gateway_name: Option<String>,
    pub monitor_mode: Option<bool>,
    pub block_failures: Option<bool>,
    pub anonymize_input: Option<bool>,
}

#[serde_inline_default]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZscalerAIGuardConfigModel {
    pub policy_id: Option<i64>,
    #[serde_inline_default(Some(false))]
    pub send_user_api_key_alias: Option<bool>,
    #[serde_inline_default(Some(false))]
    pub send_user_api_key_user_id: Option<bool>,
    #[serde_inline_default(Some(false))]
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
