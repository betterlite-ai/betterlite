use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::message_content::Message;
use betterlite_prompts::message_fields::last_user::get_last_user_message;
use error::ModerationError;
use logging::{CallTiming, RequestMetadata};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnNull, serde_as};

mod check;
pub mod config;
pub mod error;
pub mod logging;
mod transport;
pub mod wire;

pub const DEFAULT_API_BASE: &str = "https://api.openai.com/v1";
pub const DEFAULT_SAMPLING_RATE: u32 = 5;
// LiteLLM's guardrail client defaults to httpx.Timeout(timeout=600, connect=5); the injected client sets connect_timeout from CONNECT_TIMEOUT.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const OPENAI_API_KEY_VARIABLE: &str = "OPENAI_API_KEY";

pub const SUPPORTED_EVENT_HOOKS: [EventHook; 3] = [
    EventHook::PreCall,
    EventHook::DuringCall,
    EventHook::PostCall,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EventHook {
    #[serde(rename = "pre_call")]
    PreCall,
    #[serde(rename = "during_call")]
    DuringCall,
    #[serde(rename = "post_call")]
    PostCall,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModerationModel {
    #[default]
    #[serde(rename = "omni-moderation-latest")]
    OmniModerationLatest,
    #[serde(rename = "text-moderation-latest")]
    TextModerationLatest,
}

#[derive(Debug, Clone, Default)]
pub struct ModerationParams {
    pub guardrail_name: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub model: Option<ModerationModel>,
    pub streaming_end_of_stream_only: Option<bool>,
    pub streaming_sampling_rate: Option<u32>,
    pub timeout: Option<Duration>,
}

// litellm.api_key and litellm.openai_key, the global keys LiteLLM falls back to.
#[derive(Debug, Clone, Default)]
pub struct GlobalApiKeys {
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Inputs {
    #[serde(default)]
    pub texts: Vec<String>,
    #[serde(default)]
    pub structured_messages: Vec<Message>,
    #[serde(flatten)]
    pub rest: BTreeMap<String, GuardrailValue>,
}

#[serde_as]
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct RequestData {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub metadata: RequestMetadata,
    #[serde(flatten)]
    pub rest: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone)]
pub struct OpenAiModerationGuardrail {
    pub guardrail_name: String,
    pub api_key: SecretString,
    pub api_base: String,
    pub model: ModerationModel,
    pub streaming_end_of_stream_only: bool,
    pub streaming_sampling_rate: u32,
    pub timeout: Duration,
    pub client: reqwest::Client,
    pub clock: fn() -> Duration,
}

impl OpenAiModerationGuardrail {
    pub fn new<L>(
        params: ModerationParams,
        global_keys: GlobalApiKeys,
        lookup: &L,
        client: reqwest::Client,
        clock: fn() -> Duration,
    ) -> Result<Self, ModerationError>
    where
        L: EnvironmentLookup,
    {
        let environment_key = lookup
            .lookup(OPENAI_API_KEY_VARIABLE)
            .map(SecretString::from);
        let api_key = [
            params.api_key,
            environment_key,
            global_keys.api_key,
            global_keys.openai_key,
        ]
        .into_iter()
        .flatten()
        .find(|key| !key.expose_secret().is_empty())
        .ok_or(ModerationError::MissingApiKey)?;
        let api_base = params
            .api_base
            .filter(|base| !base.is_empty())
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        tracing::debug!(
            guardrail_name = params.guardrail_name,
            "Initialized OpenAI Moderation Guardrail"
        );
        Ok(Self {
            guardrail_name: params.guardrail_name,
            api_key,
            api_base,
            model: params.model.unwrap_or_default(),
            streaming_end_of_stream_only: params.streaming_end_of_stream_only.unwrap_or(false),
            streaming_sampling_rate: params
                .streaming_sampling_rate
                .unwrap_or(DEFAULT_SAMPLING_RATE),
            timeout: params.timeout.unwrap_or(DEFAULT_TIMEOUT),
            client,
            clock,
        })
    }

    pub async fn apply_guardrail(
        &self,
        inputs: Inputs,
        request_data: &mut RequestData,
    ) -> Result<Inputs, ModerationError> {
        let started = (self.clock)();
        let outcome = self.moderate(&inputs, &mut request_data.metadata).await;
        let timing = CallTiming {
            started,
            ended: (self.clock)(),
        };
        logging::process_outcome(
            &self.guardrail_name,
            outcome.as_ref().map(drop),
            &mut request_data.metadata,
            timing,
        );
        outcome.map(|()| inputs)
    }

    async fn moderate(
        &self,
        inputs: &Inputs,
        metadata: &mut RequestMetadata,
    ) -> Result<(), ModerationError> {
        let text_to_moderate = get_last_user_message(&inputs.structured_messages)
            .or_else(|| Some(inputs.texts.join("\n")))
            .filter(|text| !text.is_empty());
        let Some(text) = text_to_moderate else {
            tracing::debug!("OpenAI Moderation: No text content to moderate in inputs");
            return Ok(());
        };
        let moderation_response = transport::async_make_request(self, &text).await?;
        let verdict = check::check_moderation_result(&moderation_response);
        metadata.openai_moderation_response = Some(moderation_response);
        verdict
    }
}

mod tests;
