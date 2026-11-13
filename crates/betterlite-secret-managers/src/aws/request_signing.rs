use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::string::FromUtf8Error;
use std::time::Duration;

use aws_credential_types::Credentials;
use aws_sigv4::http_request::SigningError;
use aws_sigv4::sign::v4::signing_params::BuildError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use http::{HeaderMap, StatusCode};
use secrecy::{SecretSlice, SecretString};
use serde::Serialize;
use thiserror::Error;

mod optional_params;
mod post_action;
mod prepare_request;
mod sigv4;

#[derive(Debug, Default)]
pub struct AwsOptionalParams {
    pub aws_role_name: Option<String>,
    pub aws_session_name: Option<String>,
    pub aws_region_name: Option<String>,
    pub aws_external_id: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>,
    pub aws_access_key_id: Option<SecretString>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
    pub aws_bedrock_runtime_endpoint: Option<String>,
}

#[derive(Debug)]
pub struct AwsCredentialsInfo {
    pub aws_region_name: String,
    pub aws_bedrock_runtime_endpoint: Option<String>,
    pub credentials: Credentials,
}

pub trait CredentialResolver {
    type Error: Error + Send + Sync + 'static;

    fn resolve_credentials(
        &self,
        optional_params: &mut AwsOptionalParams,
    ) -> Result<AwsCredentialsInfo, Self::Error>;
}

// The type of a request that sends only the default body, so it can never be built
#[derive(Debug, Serialize)]
pub enum NoBody {}

pub struct ActionRequest<'borrowed, Body> {
    pub action: &'borrowed str,
    pub secret_name: &'borrowed str,
    pub request_data: Option<&'borrowed Body>,
}

// CreateSecret and UpdateSecret carry SecretString in request_data, so the body is never printed
impl<Body> fmt::Debug for ActionRequest<'_, Body> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionRequest")
            .field("action", &self.action)
            .field("secret_name", &self.secret_name)
            .field("has_request_data", &self.request_data.is_some())
            .finish()
    }
}

#[derive(Debug)]
pub struct PreparedRequest {
    pub endpoint_url: String,
    pub headers: HeaderMap,
    pub body: SecretSlice<u8>,
}

pub struct TransportRequest<'borrowed> {
    pub url: &'borrowed str,
    pub headers: HeaderMap,
    pub body: &'borrowed [u8],
    pub timeout: Duration,
    pub max_response_bytes: usize,
}

// PutSecretValue carries SecretString in the body, so only its length is printed
impl fmt::Debug for TransportRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransportRequest")
            .field("url", &self.url)
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("body_bytes", &self.body.len())
            .field("timeout", &self.timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
}

#[derive(Debug)]
pub struct TransportResponse {
    pub status: StatusCode,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum TransportError<Failure>
where
    Failure: Error + 'static,
{
    #[error("the request timed out")]
    Timeout,
    #[error("the response is larger than the allowed size")]
    ResponseTooLarge,
    #[error("the request failed: {0}")]
    Failed(#[source] Failure),
}

pub type TransportFuture<'request, Failure> = Pin<
    Box<dyn Future<Output = Result<TransportResponse, TransportError<Failure>>> + Send + 'request>,
>;

pub trait AwsTransport: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn post<'request>(
        &'request self,
        request: TransportRequest<'request>,
    ) -> TransportFuture<'request, Self::Error>;
}

#[derive(Debug, Error)]
pub enum AwsRequestError<ResolverError, TransportFailure>
where
    ResolverError: Error + 'static,
    TransportFailure: Error + 'static,
{
    #[error(transparent)]
    Credentials(ResolverError),
    #[error("failed to serialize the Secrets Manager request body: {0}")]
    Body(#[source] serde_json::Error),
    #[error("failed to build the SigV4 signing parameters: {0}")]
    SigningParams(#[source] BuildError),
    #[error("failed to sign the Secrets Manager request: {0}")]
    Signing(#[source] SigningError),
    #[error("signed header name is invalid: {0}")]
    HeaderName(#[source] InvalidHeaderName),
    #[error("signed header value is invalid: {0}")]
    HeaderValue(#[source] InvalidHeaderValue),
    #[error("HTTP error occurred: {text}")]
    Http { text: String },
    #[error("Timeout error occurred")]
    Timeout,
    #[error("Secrets Manager request failed: {0}")]
    Transport(#[source] TransportFailure),
    #[error("Secrets Manager response exceeds {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("Secrets Manager error text is not UTF-8: {0}")]
    ErrorText(#[source] FromUtf8Error),
    #[error("Secrets Manager response does not match the expected shape: {0}")]
    Response(#[source] serde_json::Error),
}

#[path = "request_signing/aws_request_signing/tests.rs"]
mod aws_request_signing;
