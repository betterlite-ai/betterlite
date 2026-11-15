use std::error::Error as StdError;
use std::time::Duration;

use betterlite_models::identity::provider_id::ProviderId;
use secrecy::SecretString;
use thiserror::Error;

use crate::interactions::base::{
    CallFailure, ClientError, ErrorClasses, Logging, Params, PreCallArguments, PreparedRequest,
    RequestFailure, RequestHeaders, UpstreamResponse, api_base, handle_error,
};

pub mod asynchronous;

pub mod synchronous;

pub(crate) mod tests;

pub struct EnvironmentRequest<'request> {
    pub headers: &'request RequestHeaders,
    pub model: &'request str,
    pub litellm_params: Option<&'request Params>,
}

pub struct InteractionIdRequest<'request> {
    pub interaction_id: &'request str,
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
    pub headers: &'request RequestHeaders,
}

pub struct GetInteractionRequest<'request> {
    pub interaction_id: &'request str,
    pub litellm_params: &'request Params,
    pub provider: ProviderId,
    pub extra_headers: RequestHeaders,
    // LiteLLM's GET sends no deadline of its own, so this never reaches the request
    pub timeout: Option<Duration>,
}

pub trait GetInteractionConfig: ErrorClasses {
    type Output;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, Self::Error>;

    fn transform_get_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_get_response(
        &self,
        response: &UpstreamResponse,
        logging: &mut Logging,
    ) -> Result<Self::Output, Self::Error>;
}

#[derive(Debug, Error)]
pub enum GetInteractionError<ProviderError>
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

// validate_environment, transform_get_interaction_request and the pre-call record both transports share
pub(crate) fn prepare<C>(
    config: &C,
    logging: &mut Logging,
    request: &GetInteractionRequest<'_>,
) -> Result<(RequestHeaders, PreparedRequest), GetInteractionError<C::Error>>
where
    C: GetInteractionConfig,
{
    let base = api_base(request.litellm_params).map_err(GetInteractionError::Parameters)?;
    let headers = config
        .validate_environment(&EnvironmentRequest {
            headers: &request.extra_headers,
            model: "",
            litellm_params: Some(request.litellm_params),
        })
        .map_err(GetInteractionError::Provider)?;
    let prepared = config
        .transform_get_request(&InteractionIdRequest {
            interaction_id: request.interaction_id,
            api_base: base,
            litellm_params: request.litellm_params,
            headers: &headers,
        })
        .map_err(GetInteractionError::Provider)?;
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

pub(crate) fn handle_failure<C>(
    config: &C,
    failure: RequestFailure,
) -> GetInteractionError<C::Error>
where
    C: GetInteractionConfig,
{
    match handle_error(config, failure) {
        CallFailure::Provider(error) => GetInteractionError::Provider(error),
        CallFailure::PassedThrough(passed) => GetInteractionError::Transport(passed),
    }
}

pub(crate) fn finish<C>(
    config: &C,
    logging: &mut Logging,
    response: &UpstreamResponse,
) -> Result<C::Output, GetInteractionError<C::Error>>
where
    C: GetInteractionConfig,
{
    config
        .transform_get_response(response, logging)
        .map_err(GetInteractionError::Provider)
}
