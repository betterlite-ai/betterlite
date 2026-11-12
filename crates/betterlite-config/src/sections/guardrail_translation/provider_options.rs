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
#[serde(default)]
pub struct LakeraV2GuardrailConfigModel {
    #[serde(
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub project_id: Option<String>,
    pub payload: Option<bool>,
    pub breakdown: Option<bool>,
    pub metadata: Option<BTreeMap<String, GuardrailValue>>,
    pub dev_info: Option<bool>,
    pub on_flagged: Option<LakeraOnFlagged>,
    pub advisory_system_message: Option<String>,
}

impl Default for LakeraV2GuardrailConfigModel {
    fn default() -> Self {
        Self {
            api_key: None,
            api_base: None,
            project_id: None,
            payload: Some(true),
            breakdown: Some(true),
            metadata: None,
            dev_info: Some(true),
            on_flagged: Some(LakeraOnFlagged::Block),
            advisory_system_message: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LassoGuardrailConfigModel {
    pub lasso_user_id: Option<String>,
    pub lasso_conversation_id: Option<String>,
    pub mask: Option<bool>,
}

impl Default for LassoGuardrailConfigModel {
    fn default() -> Self {
        Self {
            lasso_user_id: None,
            lasso_conversation_id: None,
            mask: Some(false),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeepKeepGuardrailConfigModel {
    pub deepkeep_firewall_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PillarGuardrailConfigModel {
    pub on_flagged_action: Option<String>,
    pub async_mode: Option<bool>,
    pub persist_session: Option<bool>,
    pub include_scanners: Option<bool>,
    pub include_evidence: Option<bool>,
}

impl Default for PillarGuardrailConfigModel {
    fn default() -> Self {
        Self {
            on_flagged_action: Some("monitor".to_owned()),
            async_mode: None,
            persist_session: None,
            include_scanners: Some(true),
            include_evidence: Some(true),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NomaGuardrailConfigModel {
    pub use_v2: Option<bool>,
    pub application_id: Option<String>,
    pub gateway_name: Option<String>,
    pub monitor_mode: Option<bool>,
    pub block_failures: Option<bool>,
    pub anonymize_input: Option<bool>,
}

impl Default for NomaGuardrailConfigModel {
    fn default() -> Self {
        Self {
            use_v2: Some(false),
            application_id: None,
            gateway_name: None,
            monitor_mode: None,
            block_failures: None,
            anonymize_input: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ZscalerAIGuardConfigModel {
    pub policy_id: Option<i64>,
    pub send_user_api_key_alias: Option<bool>,
    pub send_user_api_key_user_id: Option<bool>,
    pub send_user_api_key_team_id: Option<bool>,
}

impl Default for ZscalerAIGuardConfigModel {
    fn default() -> Self {
        Self {
            policy_id: None,
            send_user_api_key_alias: Some(false),
            send_user_api_key_user_id: Some(false),
            send_user_api_key_team_id: Some(false),
        }
    }
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
