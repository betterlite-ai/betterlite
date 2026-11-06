use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use secrecy::SecretString;
use serde::Deserialize;
use serde_with::{DefaultOnNull, serde_as};

use super::{DEFAULT_API_BASE, DEFAULT_SAMPLING_RATE, ModerationModel};

#[serde_as]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct BaseModerationConfigModel {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub model: ModerationModel,
}

#[derive(Deserialize)]
struct RawConfigModel {
    model: Option<ModerationModel>,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    streaming_end_of_stream_only: Option<bool>,
    streaming_sampling_rate: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(from = "RawConfigModel")]
pub struct OpenAiModerationConfigModel {
    pub model: ModerationModel,
    pub api_key: Option<SecretString>,
    pub api_base: String,
    pub streaming_end_of_stream_only: bool,
    pub streaming_sampling_rate: u32,
}

impl From<RawConfigModel> for OpenAiModerationConfigModel {
    fn from(raw: RawConfigModel) -> Self {
        Self {
            model: raw.model.unwrap_or_default(),
            api_key: raw.api_key,
            api_base: raw.api_base.unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
            streaming_end_of_stream_only: raw.streaming_end_of_stream_only.unwrap_or(false),
            streaming_sampling_rate: raw.streaming_sampling_rate.unwrap_or(DEFAULT_SAMPLING_RATE),
        }
    }
}

impl Default for OpenAiModerationConfigModel {
    fn default() -> Self {
        Self::from(RawConfigModel {
            model: None,
            api_key: None,
            api_base: None,
            streaming_end_of_stream_only: None,
            streaming_sampling_rate: None,
        })
    }
}

impl GuardrailConfigModelIdentity for OpenAiModerationConfigModel {
    fn ui_friendly_name() -> &'static str {
        "OpenAI Moderation"
    }
}
