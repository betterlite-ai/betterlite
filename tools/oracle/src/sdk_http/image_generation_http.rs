use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_provider::api::image_generation_http::ClientKind;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_checks::validate_timeout;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod native;
mod support;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

impl Mode {
    const fn client_kind(self) -> ClientKind {
        match self {
            Self::Sync => ClientKind::Sync,
            Self::Asynchronous => ClientKind::Asynchronous,
        }
    }
}

// The litellm_params a caller would hand the SDK: the deployment key and its certificate override
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Params {
    pub api_key: Option<String>,
    pub ssl_verify: Option<bool>,
}

// The handler's inputs: the prompt, both key channels, the options dict and the three accepted-but-unused overrides
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub model: String,
    pub prompt: String,
    pub mode: Mode,
    pub api_key: Option<String>,
    #[serde(default)]
    pub litellm_params: Params,
    #[serde(default)]
    pub optional_params: ImageParams,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub extra_body: Option<ImageParams>,
    #[serde(default)]
    pub fake_stream: bool,
    pub litellm_metadata: Option<ImageParams>,
    #[serde(default)]
    pub multipart: bool,
    pub timeout_seconds: u64,
}

// What validate_environment was handed: the explicit key, the options' own header dict, and both params maps
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateEffect {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub headers: ImageParams,
    pub model: String,
    pub messages: Vec<Json>,
    pub optional_params: ImageParams,
    pub litellm_params: ImageParams,
}

// What get_complete_url was handed: the deployment's api_base and key, never the explicit key
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UrlEffect {
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub model: String,
    pub stream: Option<bool>,
}

// What logging_obj.pre_call was handed: the prompt and the empty key the handler logs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreCallEffect {
    pub input: String,
    pub api_key: String,
    pub api_base: String,
    pub headers: ImageParams,
    pub complete_input_dict: ImageParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effects {
    pub validation: ValidateEffect,
    pub url: UrlEffect,
    pub pre_call: PreCallEffect,
}

// The exception _handle_error raises, as its callers can observe it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub status: u16,
    pub synthesized: bool,
    pub message: String,
    pub headers: BTreeMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub result: Option<Json>,
    pub failure: Option<Failure>,
    pub provider_headers: Option<BTreeMap<String, String>>,
    pub effects: Effects,
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        validate_timeout(self.timeout_seconds, limits)
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        native::execute(self, api_base, env)
    }
}
