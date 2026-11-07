use std::collections::BTreeMap;
use std::error::Error;

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::error::Category;
use thiserror::Error;

use crate::aws::request_signing::{
    ActionRequest, AwsRequestError, AwsTransport, NoBody, TransportError, TransportRequest,
    TransportResponse,
};

mod async_reads;
mod primary_reads;
mod sync_primary_reads;
mod sync_reads;
mod tests;

const GET_SECRET_VALUE: &str = "GetSecretValue";
const SECRET_STRING: &str = "SecretString";

pub trait AwsBlockingTransport: AwsTransport {
    fn post_blocking(
        &self,
        request: TransportRequest<'_>,
    ) -> Result<TransportResponse, TransportError<Self::Error>>;
}

#[derive(Debug, Error)]
pub enum AwsReadError<ResolverError, TransportFailure>
where
    ResolverError: Error + 'static,
    TransportFailure: Error + 'static,
{
    #[error(transparent)]
    Request(AwsRequestError<ResolverError, TransportFailure>),
    #[error(transparent)]
    Primary(PrimarySecretError),
}

// Serde quotes the input it failed on, and that input is a secret
#[derive(Debug, Error)]
#[error("the primary secret is not a JSON object at line {line}, column {column}")]
pub struct PrimarySecretError {
    pub category: Category,
    pub line: usize,
    pub column: usize,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SecretField {
    Text(String),
    Other(IgnoredAny),
}

pub fn parse_primary_secret(
    primary_text: Option<&SecretString>,
) -> Result<BTreeMap<String, SecretString>, PrimarySecretError> {
    let Some(text) = primary_text
        .map(ExposeSecret::expose_secret)
        .filter(|text| !text.is_empty())
    else {
        return Ok(BTreeMap::new());
    };
    let fields: BTreeMap<String, SecretField> =
        serde_json::from_str(text).map_err(|source| PrimarySecretError {
            category: source.classify(),
            line: source.line(),
            column: source.column(),
        })?;
    Ok(fields
        .into_iter()
        .filter_map(|(name, field)| match field {
            SecretField::Text(value) => Some((name, SecretString::from(value))),
            SecretField::Other(_) => None,
        })
        .collect())
}

const fn get_secret_request(secret_name: &str) -> ActionRequest<'_, NoBody> {
    ActionRequest {
        action: GET_SECRET_VALUE,
        secret_name,
        request_data: None,
    }
}

fn secret_from_fields(
    secret_name: &str,
    mut fields: BTreeMap<String, SecretField>,
) -> Option<SecretString> {
    match fields.remove(SECRET_STRING) {
        Some(SecretField::Text(text)) => Some(SecretString::from(text)),
        Some(SecretField::Other(_)) | None => {
            tracing::error!(
                secret = secret_name,
                failure = "missing_secret_string",
                "failed to read the secret"
            );
            None
        }
    }
}

fn failed_secret_read(secret_name: &str, failure: &str) -> Option<SecretString> {
    tracing::error!(
        secret = secret_name,
        failure = failure,
        "failed to read the secret"
    );
    None
}

fn failed_read<ResolverError, TransportFailure>(
    secret_name: &str,
    error: AwsRequestError<ResolverError, TransportFailure>,
) -> Result<Option<SecretString>, AwsReadError<ResolverError, TransportFailure>>
where
    ResolverError: Error + 'static,
    TransportFailure: Error + 'static,
{
    match error {
        request @ (AwsRequestError::Credentials(_)
        | AwsRequestError::Body(_)
        | AwsRequestError::SigningParams(_)
        | AwsRequestError::Signing(_)
        | AwsRequestError::HeaderName(_)
        | AwsRequestError::HeaderValue(_)) => Err(AwsReadError::Request(request)),
        AwsRequestError::Http { .. } => Ok(failed_secret_read(secret_name, "http")),
        AwsRequestError::Timeout => Ok(failed_secret_read(secret_name, "timeout")),
        AwsRequestError::Transport(_) => Ok(failed_secret_read(secret_name, "transport")),
        AwsRequestError::ResponseTooLarge { .. } => {
            Ok(failed_secret_read(secret_name, "response_too_large"))
        }
        AwsRequestError::ErrorText(_) => Ok(failed_secret_read(secret_name, "error_text")),
        AwsRequestError::Response(_) => Ok(failed_secret_read(secret_name, "response")),
    }
}

fn read_reply<ResolverError, TransportFailure>(
    secret_name: &str,
    reply: Result<BTreeMap<String, SecretField>, AwsRequestError<ResolverError, TransportFailure>>,
) -> Result<Option<SecretString>, AwsReadError<ResolverError, TransportFailure>>
where
    ResolverError: Error + 'static,
    TransportFailure: Error + 'static,
{
    match reply {
        Ok(fields) => Ok(secret_from_fields(secret_name, fields)),
        Err(error) => failed_read(secret_name, error),
    }
}
