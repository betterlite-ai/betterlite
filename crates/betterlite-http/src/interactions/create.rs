use std::error::Error as StdError;
use std::time::Duration;

use betterlite_interactions::types::interaction::InteractionInput;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use crate::interactions::base::{
    CallFailure, ClientError, ErrorClasses, Logging, Params, PreCallArguments, RequestFailure,
    RequestHeaders, UpstreamResponse, api_base, handle_error,
};
use crate::interactions::get::EnvironmentRequest;
use crate::interactions::streaming::{AsyncStreamingConfig, SyncStreamingConfig};

pub mod asynchronous;

pub mod synchronous;

pub(crate) mod tests;

pub struct CreateInteractionRequest<'request> {
    pub model: Option<&'request str>,
    pub agent: Option<&'request str>,
    pub input: Option<&'request InteractionInput>,
    pub optional_params: &'request Params,
    pub litellm_params: &'request Params,
    pub provider: ProviderId,
    pub stream: bool,
    pub extra_headers: RequestHeaders,
    pub extra_body: Option<Params>,
    pub timeout: Option<Duration>,
    // litellm.request_timeout, the deadline LiteLLM falls back to when the caller names none
    pub request_timeout: Duration,
}

pub struct CompleteUrlRequest<'request> {
    pub api_base: &'request str,
    pub model: Option<&'request str>,
    pub agent: Option<&'request str>,
    pub litellm_params: &'request Params,
    pub stream: bool,
}

pub struct TransformRequest<'request> {
    pub model: Option<&'request str>,
    pub agent: Option<&'request str>,
    pub input: Option<&'request InteractionInput>,
    pub optional_params: &'request Params,
    pub litellm_params: &'request Params,
    pub headers: &'request RequestHeaders,
}

// LiteLLM returns the response for a non-streaming call and the iterator for a streaming one
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreateOutcome<Response, Stream> {
    Response(Response),
    Stream(Stream),
}

pub trait CreateInteractionConfig: SyncStreamingConfig + AsyncStreamingConfig {
    type Output;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, Self::Error>;

    fn get_complete_url(&self, request: &CompleteUrlRequest<'_>) -> Result<String, Self::Error>;

    fn transform_request(&self, request: &TransformRequest<'_>) -> Result<Params, Self::Error>;

    fn transform_response(
        &self,
        model: Option<&str>,
        response: &UpstreamResponse,
        logging: &mut Logging,
    ) -> Result<Self::Output, Self::Error>;
}

#[derive(Debug, Error)]
pub enum CreateInteractionError<ProviderError>
where
    ProviderError: StdError + 'static,
{
    #[error("the HTTP client could not be built")]
    Client(#[source] ClientError),
    #[error("the interaction parameters are not usable")]
    Parameters(#[source] ClientError),
    #[error("the interaction input could not be logged")]
    Input(#[source] serde_json::Error),
    #[error("{0}")]
    Provider(#[source] ProviderError),
    #[error(transparent)]
    Transport(RequestFailure),
}

pub(crate) struct PreparedCreate {
    pub(crate) headers: RequestHeaders,
    pub(crate) url: String,
    pub(crate) data: Params,
}

pub(crate) fn prepare<C>(
    config: &C,
    logging: &mut Logging,
    request: &CreateInteractionRequest<'_>,
) -> Result<PreparedCreate, CreateInteractionError<C::Error>>
where
    C: CreateInteractionConfig,
{
    let (headers, url) = provider_requests(config, request)?;
    let mut data = config
        .transform_request(&TransformRequest {
            model: request.model,
            agent: request.agent,
            input: request.input,
            optional_params: request.optional_params,
            litellm_params: request.litellm_params,
            headers: &headers,
        })
        .map_err(CreateInteractionError::Provider)?;
    // dict.update: an extra_body entry replaces the one transform_request built under the same name
    if let Some(extra) = &request.extra_body {
        data.extend(extra.clone());
    }
    let logged_input = match request.input {
        None => String::new(),
        Some(InteractionInput::Text(text)) => text.clone(),
        Some(input) => serde_json::to_string(input).map_err(CreateInteractionError::Input)?,
    };
    log_pre_call(logging, &logged_input, &url, &headers, &data);
    Ok(PreparedCreate { headers, url, data })
}

// LiteLLM asks the config for the environment headers first, then for the complete URL
fn provider_requests<C>(
    config: &C,
    request: &CreateInteractionRequest<'_>,
) -> Result<(RequestHeaders, String), CreateInteractionError<C::Error>>
where
    C: CreateInteractionConfig,
{
    let headers = config
        .validate_environment(&EnvironmentRequest {
            headers: &request.extra_headers,
            model: request.model.unwrap_or_default(),
            litellm_params: Some(request.litellm_params),
        })
        .map_err(CreateInteractionError::Provider)?;
    let base = api_base(request.litellm_params).map_err(CreateInteractionError::Parameters)?;
    let url = config
        .get_complete_url(&CompleteUrlRequest {
            api_base: base,
            model: request.model,
            agent: request.agent,
            litellm_params: request.litellm_params,
            stream: request.stream,
        })
        .map_err(CreateInteractionError::Provider)?;
    Ok((headers, url))
}

fn log_pre_call(
    logging: &mut Logging,
    input: &str,
    url: &str,
    headers: &RequestHeaders,
    data: &Params,
) {
    // LiteLLM logs an empty api key here, since the provider key travels in the headers
    let api_key = SecretString::from("");
    logging.record_pre_call(
        input,
        &api_key,
        PreCallArguments {
            api_base: url,
            headers,
        },
    );
    logging.model_call_details.insert(
        "complete_input_dict".to_owned(),
        JsonValue::Object(
            data.iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect::<IndexMap<String, JsonValue>>(),
        ),
    );
}

pub(crate) fn handle_failure<C>(
    config: &C,
    failure: RequestFailure,
) -> CreateInteractionError<C::Error>
where
    C: ErrorClasses,
{
    match handle_error(config, failure) {
        CallFailure::Provider(error) => CreateInteractionError::Provider(error),
        CallFailure::PassedThrough(passed) => CreateInteractionError::Transport(passed),
    }
}

pub(crate) fn finish<C>(
    config: &C,
    logging: &mut Logging,
    model: Option<&str>,
    response: &UpstreamResponse,
) -> Result<C::Output, CreateInteractionError<C::Error>>
where
    C: CreateInteractionConfig,
{
    config
        .transform_response(model, response, logging)
        .map_err(CreateInteractionError::Provider)
}
