use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_checks::validate_timeout;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod native;
mod support;

// The two session POSTs share a flow, and the SDP exchange is the third handler
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Handler {
    ClientSecrets,
    TranscriptionSessions,
    Calls,
}

// One handler call: the credentials, the JSON session body or the SDP offer, and the caller's headers and timeout
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub handler: Handler,
    // a provider config whose URL and headers echo the call, or none for the OpenAI-style fallback
    #[serde(default)]
    pub echo_config: bool,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub ephemeral_key: Option<String>,
    #[serde(default)]
    pub request_data: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub sdp: Vec<u8>,
    #[serde(default)]
    pub session: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub api_version: Option<String>,
    pub timeout_seconds: u64,
}

// The raw response a handler hands back, or its exception, and the pre-call details when one was recorded
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub result: Option<Answer>,
    pub failure: Option<Failure>,
    pub logged: Option<Logged>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub status: u16,
    pub headers: BTreeMap<String, Json>,
    pub body: Vec<u8>,
}

// What a caller catches: the exception class, its status, message and headers, and the synthesized marker
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub class: String,
    pub status: Option<u16>,
    pub message: String,
    pub headers: BTreeMap<String, Json>,
    pub synthesized: Option<bool>,
}

// The pre-call details both sides store: the input, the URL, the header names, the body and the calls session
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Logged {
    pub input: Json,
    pub api_base: String,
    pub header_names: Vec<String>,
    pub complete_input_dict: Option<Json>,
    pub session: Option<Json>,
    pub litellm_api_base: String,
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        validate_timeout(self.timeout_seconds, limits)?;
        let (present, message) = match self.handler {
            Handler::Calls => (
                self.ephemeral_key.is_some(),
                "a calls handler needs an ephemeral key",
            ),
            Handler::ClientSecrets | Handler::TranscriptionSessions => (
                self.request_data.is_some(),
                "a session handler needs a request body",
            ),
        };
        if present {
            return Ok(());
        }
        Err(SdkHttpError::Invalid(String::from(message)))
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        native::execute(self, api_base, env)
    }
}
