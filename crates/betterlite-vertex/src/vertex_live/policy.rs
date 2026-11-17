use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use crate::gemini_live_setup::error::LiveSetupError;
use crate::gemini_live_setup::wire::{
    AutomaticActivityDetection, Envelope, GenerationConfig, LiveSetup, RealtimeInputConfig,
};

use super::error::VertexLiveError;

const GLOBAL_HOST: &str = "aiplatform.googleapis.com";
const LIVE_RPC_PATH: &str = "/ws/google.cloud.aiplatform.v1.LlmBidiService/BidiGenerateContent";
pub(super) const DEFAULT_SILENCE_DURATION_MS: u64 = 800;

pub struct VertexLivePolicy {
    pub project: String,
    pub region: String,
    pub access_token: SecretString,
}

impl VertexLivePolicy {
    #[must_use]
    pub fn complete_url(&self, api_base: Option<&str>) -> String {
        if let Some(base) = api_base {
            let converted = base
                .trim_end_matches('/')
                .replace("https://", "wss://")
                .replace("http://", "ws://");
            return format!("{converted}{LIVE_RPC_PATH}");
        }
        if self.region == "global" {
            return format!("wss://{GLOBAL_HOST}{LIVE_RPC_PATH}");
        }
        let region = &self.region;
        format!("wss://{region}-aiplatform.googleapis.com{LIVE_RPC_PATH}")
    }

    #[must_use]
    pub fn environment(&self, headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let mut environment = headers.clone();
        let token = self.access_token.expose_secret();
        environment.insert("Authorization".to_owned(), format!("Bearer {token}"));
        if !self.project.is_empty() {
            environment.insert("x-goog-user-project".to_owned(), self.project.clone());
        }
        environment
    }

    pub fn session_configuration_request(&self, model: &str) -> Result<String, VertexLiveError> {
        let setup = LiveSetup {
            model: Some(self.model_path(model)),
            generation_config: GenerationConfig {
                temperature: None,
                max_output_tokens: None,
                response_modalities: Some(vec!["AUDIO".to_owned()]),
                speech_config: None,
            },
            system_instruction: None,
            tools: None,
            tool_config: None,
            realtime_input_config: Some(RealtimeInputConfig {
                automatic_activity_detection: Some(AutomaticActivityDetection {
                    disabled: Some(false),
                    prefix_padding_ms: None,
                    silence_duration_ms: Some(Json::Number(Number::from(
                        DEFAULT_SILENCE_DURATION_MS,
                    ))),
                    start_of_speech_sensitivity: None,
                    end_of_speech_sensitivity: None,
                }),
            }),
            session_resumption: None,
            session_resumption_config: None,
            context_window_compression: None,
            input_audio_transcription: Some(Object::new()),
            output_audio_transcription: Some(Object::new()),
        };
        Ok(serde_json::to_string(&Envelope { setup: &setup }).map_err(LiveSetupError::Encode)?)
    }

    pub(super) fn model_path(&self, model: &str) -> String {
        let project = &self.project;
        let region = &self.region;
        format!("projects/{project}/locations/{region}/publishers/google/models/{model}")
    }
}
