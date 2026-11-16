use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::error::LiveSetupError;
use super::session::MaxResponseTokens;
use crate::gemini_tools::records::{Tool, ToolConfig};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSetup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "GenerationConfig::is_empty")]
    pub generation_config: GenerationConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<Instruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<ToolConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realtime_input_config: Option<RealtimeInputConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_resumption: Option<Object>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_resumption_config: Option<Object>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window_compression: Option<Object>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_audio_transcription: Option<Object>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_audio_transcription: Option<Object>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<MaxResponseTokens>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_modalities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech_config: Option<Object>,
}

impl GenerationConfig {
    const fn is_empty(&self) -> bool {
        self.temperature.is_none()
            && self.max_output_tokens.is_none()
            && self.response_modalities.is_none()
            && self.speech_config.is_none()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Instruction {
    Text(String),
    Content(SystemInstruction),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SystemInstruction {
    pub role: String,
    pub parts: Vec<InstructionPart>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InstructionPart {
    pub text: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeInputConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automatic_activity_detection: Option<AutomaticActivityDetection>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomaticActivityDetection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix_padding_ms: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silence_duration_ms: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_of_speech_sensitivity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_of_speech_sensitivity: Option<String>,
}

#[derive(Serialize)]
struct Envelope<'setup> {
    setup: &'setup LiveSetup,
}

pub(crate) fn encode_envelope(setup: &LiveSetup) -> Result<String, LiveSetupError> {
    serde_json::to_string(&Envelope { setup }).map_err(LiveSetupError::Encode)
}
