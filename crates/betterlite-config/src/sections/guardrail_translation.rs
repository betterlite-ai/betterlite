crate::sections::stub_section!();

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use super::content_filter_records::ConfidenceThreshold;
use crate::sections::guardrail_identity::item::GuardrailValue;

pub mod bedrock_checks;
pub mod bedrock_streaming;
pub mod provider_options;

use bedrock_checks::{
    BedrockChecksConfigWire, BedrockContentFilterModel, BedrockPromptAttackModel,
    BedrockSensitiveInfoModel,
};
use bedrock_streaming::ChunkBudgetChars;
use provider_options::{
    LakeraOnFlagged, default_breakdown, default_dev_info, default_include_evidence,
    default_include_scanners, default_on_flagged, default_on_flagged_action, default_payload,
    default_send_team_id, default_send_user_alias, default_send_user_id, default_use_v2,
};

// serde's library adapter builds this private, non-Debug value only while a model is serialized
#[derive(Serialize)]
#[serde(transparent)]
pub(crate) struct ExposedSecret(String);

impl From<&SecretString> for ExposedSecret {
    fn from(secret: &SecretString) -> Self {
        Self(secret.expose_secret().to_owned())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "BedrockChecksConfigWire")]
pub struct BedrockChecksConfigModel {
    #[serde(rename = "contentFilter")]
    pub content_filter: Option<BedrockContentFilterModel>,
    #[serde(rename = "promptAttack")]
    pub prompt_attack: Option<BedrockPromptAttackModel>,
    #[serde(rename = "sensitiveInformation")]
    pub sensitive_information: Option<BedrockSensitiveInfoModel>,
}

impl BedrockChecksConfigModel {
    // LiteLLM's _require_at_least_one_check model validator
    pub const fn require_at_least_one_check(
        &self,
    ) -> Result<(), bedrock_checks::BedrockChecksError> {
        if self.content_filter.is_none()
            && self.prompt_attack.is_none()
            && self.sensitive_information.is_none()
        {
            return Err(bedrock_checks::BedrockChecksError::NoCheckEnabled);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BedrockGuardrailConfigModel {
    #[serde(rename = "guardrailIdentifier")]
    pub guardrail_identifier: Option<String>,
    #[serde(rename = "guardrailVersion")]
    pub guardrail_version: Option<String>,
    pub disable_exception_on_block: Option<bool>,
    pub aws_region_name: Option<String>,
    pub aws_access_key_id: Option<String>,
    #[serde(
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub aws_secret_access_key: Option<SecretString>,
    #[serde(
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub aws_session_token: Option<SecretString>,
    pub aws_session_name: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_role_name: Option<String>,
    #[serde(
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>,
    pub aws_external_id: Option<String>,
    pub aws_bedrock_runtime_endpoint: Option<String>,
    pub checks: Option<BedrockChecksConfigModel>,
    pub content_filter_threshold: Option<ConfidenceThreshold>,
    pub prompt_attack_threshold: Option<ConfidenceThreshold>,
    pub pii_confidence_threshold: Option<ConfidenceThreshold>,
    pub chunk_budget_chars: ChunkBudgetChars,
    #[serde(rename = "contextual_grounding_from_messages")]
    pub contextual_grounding: bool,
}

impl Default for BedrockGuardrailConfigModel {
    fn default() -> Self {
        Self {
            guardrail_identifier: None,
            guardrail_version: None,
            disable_exception_on_block: Some(false),
            aws_region_name: None,
            aws_access_key_id: None,
            aws_secret_access_key: None,
            aws_session_token: None,
            aws_session_name: None,
            aws_profile_name: None,
            aws_role_name: None,
            aws_web_identity_token: None,
            aws_sts_endpoint: None,
            aws_external_id: None,
            aws_bedrock_runtime_endpoint: None,
            checks: None,
            content_filter_threshold: Some(ConfidenceThreshold(0.5)),
            prompt_attack_threshold: Some(ConfidenceThreshold(0.5)),
            pii_confidence_threshold: Some(ConfidenceThreshold(0.5)),
            chunk_budget_chars: ChunkBudgetChars(25_000),
            contextual_grounding: false,
        }
    }
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

#[path = "guardrail_translation/guardrail_provider_options/tests.rs"]
mod guardrail_provider_options;
