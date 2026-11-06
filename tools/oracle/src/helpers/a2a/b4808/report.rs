use std::collections::HashMap;
use std::fmt::Display;

use betterlite_a2a::completion_bridge::events::Event;
use betterlite_a2a::watsonx_credentials::bearer::CredentialsError;
use betterlite_a2a::watsonx_run::config::ConfigError;
use betterlite_a2a::watsonx_run::handler::RunError;
use betterlite_a2a::watsonx_run::poll::PollError;
use betterlite_a2a::watsonx_run::run_api::ApiError;
use betterlite_a2a::watsonx_transform::fake_streaming::WxoStreamResult;
use betterlite_core::agents::json::AgentJson;
use serde::Serialize;
use thiserror::Error;

use super::script::{Recorded, ScriptError};

#[derive(Debug, Error)]
pub(super) enum CallError {
    #[error(transparent)]
    Config(#[from] ConfigError<ScriptError, ScriptError>),
    #[error("the answer cannot be serialized")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Serialize)]
pub(super) struct PacedEvent<'event> {
    pub(super) event: &'event Event<WxoStreamResult>,
    pub(super) delay_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Outcome {
    Ok(AgentJson),
    Error(Failure),
}

#[derive(Serialize)]
pub(super) struct Failure {
    pub(super) kind: &'static str,
}

#[derive(Serialize)]
pub(super) struct Output {
    pub(super) outcome: Outcome,
    pub(super) requests: Vec<Recorded>,
    pub(super) sleeps: Vec<u64>,
}

const fn run_kind(error: &RunError<ScriptError, ScriptError>) -> &'static str {
    match error {
        RunError::Params(_)
        | RunError::Credentials(CredentialsError::MissingUsername | CredentialsError::Encode(_)) => {
            "params_missing"
        }
        RunError::Credentials(CredentialsError::Transport(_))
        | RunError::Api(ApiError::Transport(_))
        | RunError::Poll(PollError::Api(ApiError::Transport(_))) => "transport",
        RunError::Credentials(CredentialsError::Status { .. })
        | RunError::Api(ApiError::Status { .. })
        | RunError::Poll(PollError::Api(ApiError::Status { .. })) => "status",
        RunError::Credentials(CredentialsError::Decode(_))
        | RunError::Api(ApiError::Decode(_))
        | RunError::Poll(PollError::Api(ApiError::Decode(_))) => "decode",
        RunError::Poll(PollError::NoRunId) => "no_run_id",
        RunError::Poll(PollError::PollTimeout { .. }) => "timeout",
        RunError::Poll(PollError::NonSuccess { .. }) => "non_success",
        RunError::Sse(_) => "sse",
        RunError::Body(_) => "read",
    }
}

pub(super) const fn kind(error: &CallError) -> &'static str {
    match error {
        CallError::Config(ConfigError::MissingLitellmParams) => "config_missing",
        CallError::Config(ConfigError::LitellmParams(_)) => "config_invalid",
        CallError::Config(ConfigError::Run(run)) => run_kind(run),
        CallError::Serialize(_) => "serialize",
    }
}

pub(super) fn helper_error<M>(class: &str, message: M) -> Box<crate::helpers::HelperError>
where
    M: Display,
{
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

pub(super) fn normalize(value: &mut AgentJson, seen: &mut HashMap<String, String>) {
    match value {
        AgentJson::String(text) => {
            if text.len() == 36 && text.starts_with("00000000-0000-0000-0000-") {
                let next = format!("uuid-{}", seen.len());
                let replacement = seen.entry(text.clone()).or_insert(next);
                text.clone_from(replacement);
            }
        }
        AgentJson::Array(items) => {
            for item in items {
                normalize(item, seen);
            }
        }
        AgentJson::Object(fields) => {
            for field in fields.values_mut() {
                normalize(field, seen);
            }
        }
        AgentJson::Null | AgentJson::Bool(_) | AgentJson::Number(_) => {}
    }
}
