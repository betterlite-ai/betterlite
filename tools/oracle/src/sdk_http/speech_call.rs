use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;

mod native;
mod support;

// the harness owns the mock origin, so a case writes this where the mock URL belongs
pub(crate) const API_BASE_SENTINEL: &str = "$API_BASE";

// litellm's module-level settings speech reads, which a case sets per run
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Globals {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<String>,
    pub openai_key: Option<String>,
    pub organization: Option<String>,
    pub project: Option<String>,
    pub azure_key: Option<String>,
    pub num_retries: Option<u32>,
    pub request_timeout: Option<f64>,
}

// one speech() call with the keyword arguments the case pins
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub model: String,
    #[serde(default = "default_input")]
    pub input: String,
    #[serde(default)]
    pub voice: Option<Json>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_version: Option<String>,
    #[serde(default)]
    pub organization: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub metadata: Option<Object>,
    #[serde(default)]
    pub response_format: Option<String>,
    #[serde(default)]
    pub speed: Option<f64>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub headers: Option<Object>,
    #[serde(default)]
    pub kwargs: Object,
    // the Lyria catalog answer the case scripts for B157's selector
    #[serde(default)]
    pub lyria: bool,
    // the bytes the native B010 operation answers with; the python side reads them from the mock
    #[serde(default)]
    pub operation_body: Vec<u8>,
}

fn default_input() -> String {
    String::from("hello")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    #[serde(default)]
    pub globals: Globals,
    pub steps: Vec<Step>,
}

// the audio the SDK hands back: its status and the bytes
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinaryValue {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<BinaryValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub steps: Vec<StepOutcome>,
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.steps.is_empty() {
            return Err(SdkHttpError::Invalid(
                "a speech_call call needs at least one step".to_owned(),
            ));
        }
        for step in &self.steps {
            if let Some(seconds) = step.timeout_seconds
                && (seconds == 0 || seconds > limits.max_execution.as_secs())
            {
                return Err(SdkHttpError::Invalid(
                    "timeout_seconds must be between 1 and the execution limit".to_owned(),
                ));
            }
            if step.model.is_empty() {
                return Err(SdkHttpError::Invalid(
                    "a speech_call step needs a model".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        native::execute(self, api_base, env)
    }
}
