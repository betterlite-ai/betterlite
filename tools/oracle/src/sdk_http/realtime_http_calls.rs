use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod native;
mod result;
mod support;

// The three decorated SDK methods, one per realtime HTTP endpoint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    #[default]
    ClientSecret,
    TranscriptionSession,
    Calls,
}

// One SDK call: the routing hints, the raw inputs each method forwards, and the litellm globals a case sets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Call {
    pub operation: Operation,
    pub settings: Settings,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub session: Option<Json>,
    pub expires_after: Option<Json>,
    pub transcription_session: Option<Json>,
    pub sdp: Vec<u8>,
    pub ephemeral_key: String,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub api_version: Option<String>,
}

// The litellm_settings the SDK reads as its module globals, loaded through the real config loader.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub openai_key: Option<String>,
}

impl Settings {
    fn document(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&ConfigDocument {
            litellm_settings: SettingsDocument {
                api_base: self.api_base.as_deref(),
                api_key: self.api_key.as_deref(),
                openai_key: self.openai_key.as_deref(),
            },
        })
    }
}

#[derive(Serialize)]
struct ConfigDocument<'settings> {
    litellm_settings: SettingsDocument<'settings>,
}

#[derive(Serialize)]
struct SettingsDocument<'settings> {
    #[serde(skip_serializing_if = "Option::is_none")]
    api_base: Option<&'settings str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<&'settings str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    openai_key: Option<&'settings str>,
}

// The raw response or the exception a caller catches, and the pre-call details the logger recorded.
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

// What a caller catches: the exception class, its status, message and headers, and the synthesized marker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub class: String,
    pub status: Option<u16>,
    pub message: String,
    pub headers: BTreeMap<String, Json>,
    pub synthesized: Option<bool>,
}

// The pre-call details: the raw optional params, the resolved routing and the handler's session defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Logged {
    pub model: String,
    pub custom_llm_provider: String,
    pub optional_params: Json,
    pub litellm_api_base: String,
    pub session: Option<Json>,
}

impl CallKind for Call {
    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        native::execute(self, api_base, env)
    }
}
