use std::collections::BTreeMap;
use std::error::Error as StdError;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use secrecy::SecretString;
use thiserror::Error;

use crate::proxy::limits::{ReadError, TransportLimits};
use crate::tls::TlsError;

pub(crate) mod clients;

pub(crate) mod transport;

pub(crate) mod tests;

pub type Params = BTreeMap<String, JsonValue>;
pub type RequestHeaders = BTreeMap<String, SecretString>;
pub type ResponseHeaders = BTreeMap<String, String>;

// the provider get_async_httpx_client builds for when litellm_params names none
pub const DEFAULT_PROVIDER: ProviderId = ProviderId::Gemini;
// the crate's buffered body cap, so a provider answer cannot grow without bound
pub const MAX_RESPONSE_BYTES: usize = TransportLimits::DEFAULT.body_bytes;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreparedRequest {
    pub url: String,
    pub params: Params,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpstreamResponse {
    pub status_code: u16,
    pub headers: ResponseHeaders,
    pub body: Vec<u8>,
}

// the arguments LiteLLM's pre_call receives as additional_args
pub struct PreCallArguments<'call> {
    pub api_base: &'call str,
    pub headers: &'call RequestHeaders,
}

#[derive(Clone, Debug)]
pub struct PreCall {
    pub input: String,
    pub api_key: SecretString,
    pub api_base: String,
    pub headers: RequestHeaders,
}

#[derive(Clone, Debug, Default)]
pub struct Logging {
    pub model_call_details: Params,
    pub pre_call: Option<PreCall>,
}

impl Logging {
    // LiteLLM's Logging.pre_call: the call input and the arguments the request is about to use
    pub fn record_pre_call(&mut self, input: &str, api_key: &str, arguments: PreCallArguments<'_>) {
        self.pre_call = Some(PreCall {
            input: input.to_owned(),
            api_key: SecretString::from(api_key),
            api_base: arguments.api_base.to_owned(),
            headers: arguments.headers.clone(),
        });
    }
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Tls(#[from] TlsError),
    #[error("could not build the HTTP client")]
    Build(#[source] reqwest::Error),
    #[error("litellm_params.{field} must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("\"{provider}\" is not a provider betterlite knows")]
    UnknownProvider { provider: String },
}

#[derive(Debug, Error)]
pub enum RequestFailure {
    #[error("the upstream answered with status {status_code}")]
    Status {
        status_code: u16,
        headers: ResponseHeaders,
        body: String,
    },
    #[error("the request URL is not valid")]
    Url(#[source] url::ParseError),
    #[error("the request header name is not valid")]
    HeaderName(#[source] reqwest::header::InvalidHeaderName),
    #[error("the request header value is not valid")]
    HeaderValue(#[source] reqwest::header::InvalidHeaderValue),
    #[error("the {name} response header is not valid UTF-8")]
    ResponseHeader { name: String },
    #[error("litellm_params.{field} cannot be a query parameter")]
    QueryValue { field: String },
    #[error("the request to the provider failed")]
    Transport(#[source] reqwest::Error),
    #[error("the response body is larger than {limit} bytes")]
    BodyTooLarge { limit: usize },
    #[error("the response body could not be read")]
    Body(#[source] std::io::Error),
}

impl From<ReadError> for RequestFailure {
    fn from(failure: ReadError) -> Self {
        match failure {
            ReadError::TooLarge { limit } => RequestFailure::BodyTooLarge { limit },
            ReadError::Failed(source) => RequestFailure::Transport(source),
        }
    }
}

#[derive(Debug, Error)]
pub enum CallFailure<ProviderError>
where
    ProviderError: StdError + 'static,
{
    #[error("{0}")]
    Provider(#[source] ProviderError),
    #[error(transparent)]
    PassedThrough(RequestFailure),
}

pub trait ErrorClasses: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: &ResponseHeaders,
    ) -> Self::Error;
}

// _handle_error: a failing status becomes the provider's own error class, everything else passes through
#[must_use]
pub fn handle_error<C>(config: &C, failure: RequestFailure) -> CallFailure<C::Error>
where
    C: ErrorClasses,
{
    match failure {
        RequestFailure::Status {
            status_code,
            headers,
            body,
        } => CallFailure::Provider(config.get_error_class(&body, status_code, &headers)),
        failure => CallFailure::PassedThrough(failure),
    }
}
