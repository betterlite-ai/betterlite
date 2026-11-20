use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_openai::openai_base_config::Object;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_checks::validate_timeout;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod fixtures;
mod native;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderConfig {
    #[default]
    Present,
    Missing,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    #[serde(default)]
    pub data: Option<Object>,
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub files: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub content_type: Option<String>,
}

// Both handler modes over the fixture provider config, with the request and response a real provider would answer
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub mode: Mode,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub optional_params: Object,
    #[serde(default)]
    pub litellm_params: Object,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub response: Option<TranscriptionResponse>,
    #[serde(default)]
    pub supports_subtitle_synthesis: bool,
    #[serde(default)]
    pub provider_config: ProviderConfig,
    #[serde(default)]
    pub atranscription: bool,
    #[serde(default)]
    pub max_retries: u32,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub observe_headers: Vec<String>,
}

// The reply both handlers return, read through the fields and hidden headers a caller can see
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub text: Option<String>,
    pub words: Option<Json>,
    pub headers: BTreeMap<String, String>,
}

// The provider error _handle_error raises, as its callers can observe it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub status: u16,
    pub synthesized: bool,
    pub message: String,
    pub headers: BTreeMap<String, Json>,
}

// A missing provider config raises before any HTTP, so its class and text are all that reach the caller
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Raised {
    pub class: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub result: Option<View>,
    pub failure: Option<Failure>,
    pub raised: Option<Raised>,
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.transform.data.is_some() && self.transform.raw.is_some() {
            return Err(SdkHttpError::Invalid(
                "transform.data and transform.raw are exclusive".to_owned(),
            ));
        }
        if self.transform.raw.is_some() && self.transform.files.is_some() {
            return Err(SdkHttpError::Invalid(
                "transform.raw cannot be sent with transform.files".to_owned(),
            ));
        }
        if let Some(seconds) = self.timeout_seconds {
            validate_timeout(seconds, limits)?;
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
