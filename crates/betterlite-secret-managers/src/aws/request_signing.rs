use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::string::FromUtf8Error;

use aws_credential_types::Credentials;
use aws_sigv4::http_request::SigningError;
use aws_sigv4::sign::v4::signing_params::BuildError;
use reqwest::header::{HeaderMap, InvalidHeaderName, InvalidHeaderValue};
use secrecy::{SecretSlice, SecretString};
use serde_json::value::RawValue;
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

#[derive(Debug, Clone, Copy)]
pub struct ActionRequest<'borrowed> {
    pub action: &'borrowed str,
    pub secret_name: &'borrowed str,
    pub request_data: Option<&'borrowed BTreeMap<String, Box<RawValue>>>,
}

#[derive(Debug)]
pub struct PreparedRequest {
    pub endpoint_url: String,
    pub headers: HeaderMap,
    pub body: SecretSlice<u8>,
}

pub struct AwsActionResponse {
    pub fields: BTreeMap<String, Box<RawValue>>,
}

impl fmt::Debug for AwsActionResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsActionResponse")
            .field("fields", &self.fields.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum AwsRequestError<ResolverError>
where
    ResolverError: Error + 'static,
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
    Transport(#[source] reqwest::Error),
    #[error("Secrets Manager response exceeds {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("Secrets Manager error text is not UTF-8: {0}")]
    ErrorText(#[source] FromUtf8Error),
    #[error("Secrets Manager response is not a JSON object: {0}")]
    Response(#[source] serde_json::Error),
}

#[path = "request_signing/aws_request_signing/tests.rs"]
mod aws_request_signing;
