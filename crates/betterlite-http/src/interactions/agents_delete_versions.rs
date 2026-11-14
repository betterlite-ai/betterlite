use std::error::Error as StdError;
use std::str::from_utf8;
use std::time::{Duration, TryFromFloatSecsError};

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use crate::interactions::base::{
    CallFailure, ClientError, ErrorClasses, Logging, Params, PreCallArguments, PreparedRequest,
    RequestFailure, RequestHeaders, UpstreamResponse, api_base, handle_error,
};

pub mod asynchronous;

pub mod synchronous;

pub struct EnvironmentRequest<'request> {
    pub headers: &'request RequestHeaders,
    pub litellm_params: &'request Params,
}

pub struct NamedAgentRequest<'request> {
    pub name: &'request str,
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
}

pub struct AgentRequest<'request> {
    pub name: &'request str,
    pub litellm_params: &'request Params,
    pub extra_headers: RequestHeaders,
    pub timeout: Option<Duration>,
    pub request_timeout: RequestTimeout,
}

#[derive(Clone, Copy)]
enum CallKind {
    Delete,
    Versions,
}

struct PreparedCall {
    method: CallKind,
    headers: RequestHeaders,
    target: PreparedRequest,
    timeout: Option<Duration>,
}

pub trait AgentsConfig: ErrorClasses {
    type DeleteOutput;
    type VersionsOutput;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, Self::Error>;

    fn transform_delete_request(
        &self,
        request: &NamedAgentRequest<'_>,
    ) -> Result<String, Self::Error>;

    fn transform_delete_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<Self::DeleteOutput, Self::Error>;

    fn transform_versions_request(
        &self,
        request: &NamedAgentRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_versions_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<Self::VersionsOutput, Self::Error>;
}

#[derive(Debug, Error)]
pub enum AgentsDeleteVersionsError<ProviderError>
where
    ProviderError: StdError + 'static,
{
    #[error("the HTTP client could not be built")]
    Client(#[source] ClientError),
    #[error("the agent parameters are not usable")]
    Parameters(#[source] ClientError),
    #[error("{0}")]
    Provider(#[source] ProviderError),
    #[error(transparent)]
    Transport(RequestFailure),
    #[error("the request timeout of {seconds} seconds is not a usable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
}

fn prepare<C>(
    config: &C,
    logging: &mut Logging,
    request: &AgentRequest<'_>,
    method: CallKind,
) -> Result<PreparedCall, AgentsDeleteVersionsError<C::Error>>
where
    C: AgentsConfig,
{
    let headers = config
        .validate_environment(&EnvironmentRequest {
            headers: &request.extra_headers,
            litellm_params: request.litellm_params,
        })
        .map_err(AgentsDeleteVersionsError::Provider)?;
    let base = api_base(request.litellm_params).map_err(AgentsDeleteVersionsError::Parameters)?;
    let named = NamedAgentRequest {
        name: request.name,
        api_base: base,
        litellm_params: request.litellm_params,
    };
    let target = request_target(config, &named, method)?;
    record_pre_call(logging, request.name, &target.url, &headers);
    // LiteLLM's GET for the versions list passes no deadline, so the client's own applies
    let timeout = match method {
        CallKind::Delete => Some(deletion_timeout(request.timeout, request.request_timeout)?),
        CallKind::Versions => None,
    };
    Ok(PreparedCall {
        method,
        headers,
        target,
        timeout,
    })
}

fn request_target<C>(
    config: &C,
    named: &NamedAgentRequest<'_>,
    method: CallKind,
) -> Result<PreparedRequest, AgentsDeleteVersionsError<C::Error>>
where
    C: AgentsConfig,
{
    match method {
        CallKind::Delete => Ok(PreparedRequest {
            url: config
                .transform_delete_request(named)
                .map_err(AgentsDeleteVersionsError::Provider)?,
            params: Params::new(),
        }),
        CallKind::Versions => config
            .transform_versions_request(named)
            .map_err(AgentsDeleteVersionsError::Provider),
    }
}

// LiteLLM logs an empty api key here, since the provider key travels in the headers
fn record_pre_call(logging: &mut Logging, name: &str, url: &str, headers: &RequestHeaders) {
    let api_key = SecretString::from("");
    logging.record_pre_call(
        name,
        &api_key,
        PreCallArguments {
            api_base: url,
            headers,
        },
    );
}

fn handle_failure<C>(config: &C, failure: RequestFailure) -> AgentsDeleteVersionsError<C::Error>
where
    C: AgentsConfig,
{
    match handle_error(config, failure) {
        CallFailure::Provider(error) => AgentsDeleteVersionsError::Provider(error),
        CallFailure::PassedThrough(passed) => AgentsDeleteVersionsError::Transport(passed),
    }
}

// LiteLLM's response transforms raise for any non-2xx answer, after its post_call has run
fn ensure_success(response: UpstreamResponse) -> Result<UpstreamResponse, RequestFailure> {
    if (200..300).contains(&response.status_code) {
        return Ok(response);
    }
    Err(RequestFailure::Status {
        status_code: response.status_code,
        headers: response.headers,
        body: reply_text(&response.body),
    })
}

fn reply_text(body: &[u8]) -> String {
    match from_utf8(body) {
        Ok(text) => text.to_owned(),
        Err(invalid) => from_utf8(body.get(..invalid.valid_up_to()).unwrap_or_default())
            .unwrap_or_default()
            .to_owned(),
    }
}

fn record_post_call(logging: &mut Logging, response: &UpstreamResponse) {
    let text = reply_text(&response.body);
    logging
        .model_call_details
        .insert("original_response".to_owned(), JsonValue::Text(text));
    logging.model_call_details.insert(
        "additional_args".to_owned(),
        JsonValue::Object(IndexMap::new()),
    );
}

// LiteLLM writes timeout or request_timeout, so an absent timeout takes B198's REQUEST_TIMEOUT value
fn deletion_timeout<ProviderError>(
    timeout: Option<Duration>,
    request_timeout: RequestTimeout,
) -> Result<Duration, AgentsDeleteVersionsError<ProviderError>>
where
    ProviderError: StdError + 'static,
{
    if let Some(given) = timeout {
        return Ok(given);
    }
    Duration::try_from_secs_f64(request_timeout.seconds).map_err(|source| {
        AgentsDeleteVersionsError::Timeout {
            seconds: request_timeout.seconds,
            source,
        }
    })
}

#[path = "agents_delete_versions/tests/client.rs"]
mod client_tests;

#[path = "agents_delete_versions/tests/delete.rs"]
mod delete_tests;

#[path = "agents_delete_versions/tests/support.rs"]
mod support;

#[path = "agents_delete_versions/tests/versions.rs"]
mod versions_tests;
