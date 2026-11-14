use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::time::Duration;

use thiserror::Error;

use crate::interactions::base::{
    CallFailure, ClientError, ErrorClasses, Params, PreparedRequest, RequestFailure,
    RequestHeaders, UpstreamResponse,
};

mod async_client_call;
mod client_call;
pub mod create_async;
mod create_call;
mod create_call_async;
pub mod create_sync;
pub mod get_async;
mod get_call;
pub mod get_sync;
pub mod list_async;
mod list_call;
pub mod list_sync;
pub mod logging;
mod read_call;
mod read_call_async;

#[path = "agents_create_read/tests/client_tests.rs"]
mod client_tests;
#[path = "agents_create_read/tests/create_tests.rs"]
mod create_tests;
#[path = "agents_create_read/tests/get_tests.rs"]
mod get_tests;
#[path = "agents_create_read/tests/list_tests.rs"]
mod list_tests;
#[path = "agents_create_read/tests/requests.rs"]
mod requests;
#[path = "agents_create_read/tests/support.rs"]
mod support;

#[derive(Debug, Error)]
pub enum AgentRequestError<ProviderError>
where
    ProviderError: StdError + 'static,
{
    #[error("the HTTP client could not be built")]
    Client(#[source] ClientError),
    #[error("the agent request parameters are not usable")]
    Parameters(#[source] ClientError),
    #[error("{0}")]
    Provider(#[source] ProviderError),
    #[error(transparent)]
    Transport(RequestFailure),
}

impl<ProviderError> From<CallFailure<ProviderError>> for AgentRequestError<ProviderError>
where
    ProviderError: StdError + 'static,
{
    fn from(failure: CallFailure<ProviderError>) -> Self {
        match failure {
            CallFailure::Provider(error) => Self::Provider(error),
            CallFailure::PassedThrough(passed) => Self::Transport(passed),
        }
    }
}

pub trait AgentsApiConfig: ErrorClasses {
    type CreateOutput;
    type ListOutput;
    type GetOutput;

    fn validate_environment(
        &self,
        headers: &BTreeMap<String, String>,
        litellm_params: &Params,
    ) -> Result<RequestHeaders, Self::Error>;

    fn get_complete_url(
        &self,
        api_base: &str,
        litellm_params: &Params,
    ) -> Result<String, Self::Error>;

    fn transform_create_request(
        &self,
        name: &str,
        litellm_params: &Params,
    ) -> Result<Params, Self::Error>;

    fn transform_create_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<Self::CreateOutput, Self::Error>;

    fn transform_list_request(
        &self,
        api_base: &str,
        litellm_params: &Params,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_list_response(
        &self,
        response: &UpstreamResponse,
    ) -> Result<Self::ListOutput, Self::Error>;

    fn transform_get_request(
        &self,
        name: &str,
        api_base: &str,
        litellm_params: &Params,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_get_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<Self::GetOutput, Self::Error>;
}

pub struct CreateAgentRequest<'request> {
    pub name: &'request str,
    pub litellm_params: &'request Params,
    pub extra_headers: BTreeMap<String, String>,
    pub extra_body: Option<Params>,
    pub timeout: Option<Duration>,
    // B198's request_timeout, the deadline LiteLLM uses when the caller names none
    pub request_timeout: Duration,
}

pub struct ListAgentsRequest<'request> {
    pub litellm_params: &'request Params,
    pub extra_headers: BTreeMap<String, String>,
    // LiteLLM's GET sends no deadline of its own, so this never reaches the request
    pub timeout: Option<Duration>,
}

pub struct GetAgentRequest<'request> {
    pub name: &'request str,
    pub litellm_params: &'request Params,
    pub extra_headers: BTreeMap<String, String>,
    // LiteLLM's GET sends no deadline of its own, so this never reaches the request
    pub timeout: Option<Duration>,
}

// httpx decodes a response body as UTF-8, then as Latin-1, so a logging record never fails on it
pub(crate) fn body_text(body: &[u8]) -> String {
    match str::from_utf8(body) {
        Ok(text) => text.to_owned(),
        Err(_not_utf8) => body.iter().map(|byte| char::from(*byte)).collect(),
    }
}
