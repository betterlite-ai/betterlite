crate::sections::stub_section!();

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_inline_default::serde_inline_default;

use super::content_filter_records::ConfidenceThreshold;
use crate::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};

pub mod bedrock_checks;
pub mod provider_options;

use bedrock_checks::{
    BedrockChecksConfigWire, BedrockContentFilterModel, BedrockPromptAttackModel,
    BedrockSensitiveInfoModel,
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

#[derive(Debug, thiserror::Error)]
pub enum BedrockConfigError {
    #[error("chunk_budget_chars must be greater than 0, got {value}")]
    ChunkBudget { value: u64 },
    #[error("streaming_sampling_rate must be at least 1, got {value}")]
    SamplingRate { value: u64 },
    #[error("the Bedrock streaming extras do not decode: {source}")]
    Extras {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64")]
pub struct ChunkBudgetChars(pub u64);

impl TryFrom<u64> for ChunkBudgetChars {
    type Error = BedrockConfigError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            return Err(BedrockConfigError::ChunkBudget { value });
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64")]
pub struct SamplingRate(pub u64);

impl TryFrom<u64> for SamplingRate {
    type Error = BedrockConfigError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            return Err(BedrockConfigError::SamplingRate { value });
        }
        Ok(Self(value))
    }
}

const STREAMING_PARAM_KEYS: [&str; 4] = [
    "streaming_buffer_until_moderated",
    "streaming_sampling_rate",
    "streaming_end_of_stream_only",
    "streaming_buffer_release_on_scan",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BedrockGuardrailStreamParams {
    #[serde(rename = "streaming_buffer_until_moderated")]
    pub buffer_until_moderated: GuardrailFlag,
    pub streaming_sampling_rate: SamplingRate,
    pub streaming_end_of_stream_only: GuardrailFlag,
    #[serde(rename = "streaming_buffer_release_on_scan")]
    pub buffer_release_on_scan: GuardrailFlag,
}

impl Default for BedrockGuardrailStreamParams {
    fn default() -> Self {
        Self {
            buffer_until_moderated: true,
            streaming_sampling_rate: SamplingRate(5),
            streaming_end_of_stream_only: false,
            buffer_release_on_scan: false,
        }
    }
}

impl BedrockGuardrailStreamParams {
    pub fn from_extras(
        extras: Option<&BTreeMap<String, GuardrailValue>>,
    ) -> Result<Self, BedrockConfigError> {
        let Some(configured) = extras else {
            return Ok(Self::default());
        };
        let mut known = BTreeMap::new();
        for key in STREAMING_PARAM_KEYS {
            match configured.get(key) {
                Some(GuardrailValue::Null) | None => {}
                Some(value) => {
                    known.insert((*key).to_owned(), value.clone());
                }
            }
        }
        if known.is_empty() {
            return Ok(Self::default());
        }
        let encoded =
            serde_json::to_value(known).map_err(|source| BedrockConfigError::Extras { source })?;
        serde_json::from_value(encoded).map_err(|source| BedrockConfigError::Extras { source })
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

#[serde_inline_default]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedrockGuardrailConfigModel {
    #[serde(rename = "guardrailIdentifier")]
    pub guardrail_identifier: Option<String>,
    #[serde(rename = "guardrailVersion")]
    pub guardrail_version: Option<String>,
    #[serde_inline_default(Some(false))]
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
    #[serde_inline_default(Some(ConfidenceThreshold(0.5)))]
    pub content_filter_threshold: Option<ConfidenceThreshold>,
    #[serde_inline_default(Some(ConfidenceThreshold(0.5)))]
    pub prompt_attack_threshold: Option<ConfidenceThreshold>,
    #[serde_inline_default(Some(ConfidenceThreshold(0.5)))]
    pub pii_confidence_threshold: Option<ConfidenceThreshold>,
    #[serde_inline_default(ChunkBudgetChars(25_000))]
    pub chunk_budget_chars: ChunkBudgetChars,
    #[serde(rename = "contextual_grounding_from_messages")]
    #[serde_inline_default(false)]
    pub contextual_grounding: bool,
}

#[path = "guardrail_translation/guardrail_provider_options/tests.rs"]
mod guardrail_provider_options;
