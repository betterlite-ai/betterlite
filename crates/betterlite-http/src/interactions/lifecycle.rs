use std::error::Error as StdError;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_models::identity::provider_id::ProviderId;
use secrecy::SecretString;
use thiserror::Error;

use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::{
    CallFailure, ClientError, ErrorClasses, Logging, Params, PreCallArguments, PreparedRequest,
    RequestFailure, RequestHeaders, ResponseHeaders, UpstreamResponse, api_base, handle_error,
};
use crate::interactions::get::{EnvironmentRequest, InteractionIdRequest};

pub mod asynchronous;

pub mod synchronous;

pub(crate) mod tests;

pub struct LifecycleRequest<'request> {
    pub interaction_id: &'request str,
    pub litellm_params: &'request Params,
    pub provider: ProviderId,
    pub extra_headers: RequestHeaders,
    pub timeout: Option<Duration>,
    // litellm.request_timeout, the deadline LiteLLM falls back to when the caller names none
    pub request_timeout: Duration,
}

pub struct InteractionsHttpHandler<'config, C, ClientType> {
    pub config: &'config C,
    pub clients: HttpClients,
    pub logging: Logging,
    // a client the caller handed in, which this crate never rebuilds and never routes through a proxy
    pub supplied: Option<ClientType>,
}

pub(crate) struct Outbound<ClientType> {
    pub(crate) client: ClientType,
    pub(crate) proxy: Option<Arc<ProxyPolicy>>,
}

pub trait InteractionLifecycleConfig: ErrorClasses {
    type DeleteOutput;

    type CancelOutput;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, Self::Error>;

    fn transform_delete_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_delete_response(
        &self,
        response: &UpstreamResponse,
        logging: &mut Logging,
        interaction_id: &str,
    ) -> Result<Self::DeleteOutput, Self::Error>;

    fn transform_cancel_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_cancel_response(
        &self,
        response: &UpstreamResponse,
        logging: &mut Logging,
    ) -> Result<Self::CancelOutput, Self::Error>;
}

#[derive(Debug, Error)]
pub enum LifecycleError<ProviderError>
where
    ProviderError: StdError + 'static,
{
    #[error("the HTTP client could not be built")]
    Client(#[source] ClientError),
    #[error("the interaction parameters are not usable")]
    Parameters(#[source] ClientError),
    #[error("{0}")]
    Provider(#[source] ProviderError),
    #[error(transparent)]
    Transport(RequestFailure),
}

pub type Fault<C> = LifecycleError<<C as ErrorClasses>::Error>;

pub type DeleteResult<C> = Result<<C as InteractionLifecycleConfig>::DeleteOutput, Fault<C>>;

pub type CancelResult<C> = Result<<C as InteractionLifecycleConfig>::CancelOutput, Fault<C>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Delete,
    Cancel,
}

pub(crate) fn prepare<C>(
    config: &C,
    logging: &mut Logging,
    request: &LifecycleRequest<'_>,
    operation: Operation,
) -> Result<(RequestHeaders, PreparedRequest), Fault<C>>
where
    C: InteractionLifecycleConfig,
{
    let base = api_base(request.litellm_params).map_err(LifecycleError::Parameters)?;
    let headers = config
        .validate_environment(&EnvironmentRequest {
            headers: &request.extra_headers,
            model: "",
            litellm_params: Some(request.litellm_params),
        })
        .map_err(LifecycleError::Provider)?;
    let id_request = InteractionIdRequest {
        interaction_id: request.interaction_id,
        api_base: base,
        litellm_params: request.litellm_params,
        headers: &headers,
    };
    let prepared = match operation {
        Operation::Delete => config.transform_delete_request(&id_request),
        Operation::Cancel => config.transform_cancel_request(&id_request),
    }
    .map_err(LifecycleError::Provider)?;
    // LiteLLM logs an empty api key here, since the provider key travels in the headers
    let api_key = SecretString::from("");
    logging.record_pre_call(
        request.interaction_id,
        &api_key,
        PreCallArguments {
            api_base: &prepared.url,
            headers: &headers,
        },
    );
    Ok((headers, prepared))
}

pub(crate) fn handle_failure<C>(config: &C, failure: RequestFailure) -> Fault<C>
where
    C: InteractionLifecycleConfig,
{
    match handle_error(config, failure) {
        CallFailure::Provider(error) => LifecycleError::Provider(error),
        CallFailure::PassedThrough(passed) => LifecycleError::Transport(passed),
    }
}

// httpx's raise_for_status passes only a 2xx answer through; every other status becomes the provider's error class with its body as the message
pub(crate) fn check_status(
    status_code: u16,
    headers: ResponseHeaders,
    body: Vec<u8>,
) -> Result<UpstreamResponse, RequestFailure> {
    if !(200..=299).contains(&status_code) {
        let message = body_text(&body);
        return Err(RequestFailure::Status {
            status_code,
            headers,
            body: message,
        });
    }
    Ok(UpstreamResponse {
        status_code,
        headers,
        body,
    })
}

// httpx's response.text: UTF-8 with errors="replace", so each invalid sequence reads as one U+FFFD
fn body_text(body: &[u8]) -> String {
    let mut text = String::with_capacity(body.len());
    for chunk in body.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}
