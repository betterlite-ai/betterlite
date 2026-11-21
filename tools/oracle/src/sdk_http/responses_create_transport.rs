use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::responses_compact_http::service::ClientKind;
use betterlite_core::api::responses_create_http::config::Params;
use betterlite_core::api::responses_create_http::service::StreamSetup;
use betterlite_models::decisions::Json;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::responses_compact_transport::{Failure, Mode};
use super::result::kinds::SdkResult;

mod config;
mod hooks;
mod native;
mod support;

// Both create handlers' inputs: the mode and identity, the typed optional params, the fake-stream value and the hooks
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub mode: Mode,
    pub model: String,
    pub input: ResponsesInput,
    #[serde(default)]
    pub optional_params: Object,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<Object>,
    #[serde(default)]
    pub timeout_seconds: Option<f64>,
    // The raw value, so an exact true is the identity check the fake-stream prep makes
    #[serde(default)]
    pub fake_stream: Option<Json>,
    #[serde(default)]
    pub litellm_metadata: Option<Params>,
    #[serde(default)]
    pub litellm_params: Params,
    // The kwargs a callback overrides async_pre_call_deployment_hook with, which only the sync handler runs
    #[serde(default)]
    pub pre_hook: Option<Params>,
    #[serde(default)]
    pub agentic: bool,
    #[serde(default)]
    pub agentic_result: Option<Json>,
}

// The iterator B927 builds is either live from the send or rebuilt from one complete response
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    Live,
    Synthetic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stream {
    pub kind: StreamKind,
    pub setup: StreamSetup,
    // The completed response a synthetic iterator rebuilds its events from
    pub response: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub result: Option<Json>,
    pub failure: Option<Failure>,
    pub stream: Option<Stream>,
}

impl Call {
    const fn client_kind(&self) -> ClientKind {
        match self.mode {
            Mode::Sync => ClientKind::Sync,
            Mode::Asynchronous => ClientKind::Asynchronous,
        }
    }
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
