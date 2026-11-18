use super::completion::HandlerFuture;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::error::RedactionError;
use http::header::InvalidHeaderName;
use http::header::InvalidHeaderValue;
use std::string::FromUtf8Error;
mod calls;
mod recording;
pub(crate) mod runtime;
mod services;

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Messages,
    Legacy,
    Count,
    BatchCreate,
    BatchRetrieve,
    BatchResults,
    Upload,
    Retrieve,
    List,
    Delete,
    Content,
    ConfigUrl,
}

#[derive(Clone, Deserialize)]
pub struct Input {
    pub action: Action,
    pub model: String,
    pub provider: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub global_base: Option<String>,
    pub anthropic_key: Option<SecretString>,
    pub global_key: Option<SecretString>,
    pub environment: super::base::Environment,
    #[serde(flatten)]
    pub settings: Settings,
    pub messages: Vec<Json>,
    pub optional_params: IndexMap<String, Json>,
    pub litellm_params: IndexMap<String, Json>,
    pub headers: BTreeMap<String, SecretString>,
    pub custom_prompt_dict: BTreeMap<String, Json>,
    pub global_prompt_dict: BTreeMap<String, Json>,
    pub stream: bool,
    pub acompletion: bool,
    pub client: Option<String>,
    pub shared_session: Option<String>,
    pub encoding: String,
    pub expected_key: Option<SecretString>,
    pub failure: Option<u16>,
    pub response: String,
    pub id: String,
    pub purpose: Option<String>,
    pub file: String,
    pub system: Option<Json>,
    pub tools: Option<Json>,
}

#[derive(Clone, Deserialize)]
pub struct Settings {
    pub disable_suffix: bool,
    pub config_present: bool,
}

#[derive(Deserialize)]
pub struct Case {
    pub inputs: Vec<Input>,
    pub expected: Vec<Observation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub result: Json,
    pub post_call: bool,
    pub post_matches: bool,
    pub request: Option<Recorded>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recorded {
    pub method: String,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: BTreeMap<String, String>,
    pub body: Json,
}

#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    #[error("fixture serialization failed")]
    Json(#[from] serde_json::Error),
    #[error("fixture logging failed")]
    Logging(#[from] LoggingStateError),
    #[error("fixture redactor failed")]
    Redactor(#[from] RedactionError),
    #[error("fixture text is not UTF-8")]
    Utf8(#[from] FromUtf8Error),
    #[error("fixture header is invalid")]
    HeaderName(#[from] InvalidHeaderName),
    #[error("fixture header is invalid")]
    HeaderValue(#[from] InvalidHeaderValue),
}

#[must_use]
pub fn execute(case: &Case) -> HandlerFuture<'_, Vec<Observation>, FixtureError> {
    Box::pin(async move {
        let mut results = Vec::new();
        for input in &case.inputs {
            results.push(calls::run(input).await?);
        }
        Ok(results)
    })
}
