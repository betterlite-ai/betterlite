use std::collections::BTreeMap;
use std::future::Future;

use betterlite_openai::openai_base_config::Object;
use betterlite_provider::api::speech_http as production;
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
    const fn client_kind(self) -> production::Mode {
        match self {
            Self::Sync => production::Mode::Sync,
            Self::Asynchronous => production::Mode::Asynchronous,
        }
    }
}

// One handler call: the client class, the input, the litellm_params overrides and the transforms the case pins
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub mode: Mode,
    pub input: String,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub optional_params: Object,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub ssl_verify: Option<bool>,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub validated: Option<BTreeMap<String, String>>,
    pub transformed: Transformed,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub supplied: Option<Mode>,
}

// The speech config's answer: text_body and binary_body are accepted and dropped, since the typed request data has no such keys
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transformed {
    #[serde(default)]
    pub dict_body: Option<Object>,
    #[serde(default)]
    pub ssml_body: Option<String>,
    #[serde(default)]
    pub text_body: Option<String>,
    #[serde(default)]
    pub binary_body: Option<String>,
    #[serde(default)]
    pub headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub steps: Vec<Step>,
}

// The exception _handle_error raises, as the handler's caller observes it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    #[serde(rename = "class")]
    pub class: String,
    pub status: u16,
    pub synthesized: bool,
    pub message: String,
    pub headers: BTreeMap<String, String>,
}

// HttpxBinaryResponseContent: the bytes, the status and the provider headers the hidden params keep
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinaryValue {
    pub status: u16,
    pub body: Vec<u8>,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    pub value: Option<BinaryValue>,
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
                "a speech_http call needs at least one step".to_owned(),
            ));
        }
        for step in &self.steps {
            if let Some(seconds) = step.timeout_seconds {
                validate_timeout(seconds, limits)?;
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
