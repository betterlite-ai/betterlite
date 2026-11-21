use std::str::FromStr as _;

use betterlite_config::sections::speech_call::Settings;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::speech::client::{SharedSession, SuppliedClient};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::{Number, Value};

use super::error::SpeechCallError;

// openai.DEFAULT_MAX_RETRIES, which litellm.num_retries falls back to when it is zero
pub const DEFAULT_MAX_RETRIES: u32 = 2;

// speech()'s arguments with the signature defaults still absent; kwargs is the rest of the call's keywords
#[derive(Debug, Clone)]
pub struct SpeechCall {
    pub model: String,
    pub input: String,
    pub voice: Option<SpeechVoice>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub project: Option<String>,
    pub custom_llm_provider: Option<String>,
    // the SDK's base_url alias for api_base
    pub base_url: Option<String>,
    pub max_retries: Option<u32>,
    pub timeout_seconds: Option<f64>,
    pub metadata: Option<Object>,
    pub response_format: Option<String>,
    pub speed: Option<Decimal>,
    pub instructions: Option<String>,
    // the named headers argument, which only the vertex branches read
    pub headers: Option<Object>,
    pub client: Option<SuppliedClient>,
    pub shared_session: Option<SharedSession>,
    pub aspeech: bool,
    pub kwargs: Object,
}

impl SpeechCall {
    // optional_params starts as the three non-None speech options, as speech() builds it
    pub fn initial_optional_params(&self) -> Result<Object, SpeechCallError> {
        let mut params = Object::new();
        if let Some(format) = &self.response_format {
            params.insert(
                String::from("response_format"),
                Value::String(format.clone()),
            );
        }
        if let Some(speed) = self.speed {
            let number = Number::from_str(&speed.to_string()).map_err(|source| {
                SpeechCallError::Internal {
                    message: source.to_string(),
                }
            })?;
            params.insert(String::from("speed"), Value::Number(number));
        }
        if let Some(instructions) = &self.instructions {
            params.insert(
                String::from("instructions"),
                Value::String(instructions.clone()),
            );
        }
        Ok(params)
    }

    // timeout=None takes litellm.request_timeout, and an explicit zero stays zero
    #[must_use]
    pub fn timeout_seconds(&self, settings: &Settings) -> f64 {
        self.timeout_seconds
            .unwrap_or(settings.request_timeout_seconds)
    }

    // max_retries=None takes litellm.num_retries or 2, while an explicit zero stays zero
    #[must_use]
    pub fn max_retries(&self, settings: &Settings) -> u32 {
        self.max_retries.unwrap_or_else(|| {
            settings
                .num_retries
                .filter(|value| *value != 0)
                .unwrap_or(DEFAULT_MAX_RETRIES)
        })
    }
}
