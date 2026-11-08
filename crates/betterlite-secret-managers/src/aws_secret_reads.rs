use std::collections::BTreeMap;
use std::error::Error;

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::error::Category;
use thiserror::Error;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsOptionalParams, AwsRequestError, AwsTransport, CredentialResolver, NoBody,
};
use crate::secret_contracts::options::SecretTimeout;

pub mod blocking;
mod tests;

const GET_SECRET_VALUE: &str = "GetSecretValue";
const SECRET_STRING: &str = "SecretString";
const BOOTSTRAP_NAMES: [&str; 5] = [
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_REGION_NAME",
    "AWS_REGION",
    "AWS_BEDROCK_RUNTIME_ENDPOINT",
];

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

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the reads need the same bounds as the async_post_action impl in post_action.rs"
)]
impl<Resolver, Transport> AwsSecretsManager<Resolver, Transport>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    pub async fn async_read_secret(
        &self,
        secret_name: &str,
        optional_params: Option<&mut AwsOptionalParams>,
        timeout: Option<SecretTimeout>,
        primary_secret_name: Option<&str>,
    ) -> Result<Option<SecretString>, AwsReadError<Resolver::Error, Transport::Error>> {
        let Some(primary) = primary_secret_name.filter(|name| !name.is_empty()) else {
            let request: ActionRequest<'_, NoBody> = ActionRequest {
                action: GET_SECRET_VALUE,
                secret_name,
                request_data: None,
            };
            let reply = self
                .async_post_action(&request, optional_params, timeout)
                .await;
            return read_reply(secret_name, reply);
        };
        self.async_read_from_primary(secret_name, primary).await
    }

    pub async fn async_read_from_primary(
        &self,
        secret_name: &str,
        primary_secret_name: &str,
    ) -> Result<Option<SecretString>, AwsReadError<Resolver::Error, Transport::Error>> {
        let request: ActionRequest<'_, NoBody> = ActionRequest {
            action: GET_SECRET_VALUE,
            secret_name: primary_secret_name,
            request_data: None,
        };
        let reply = self.async_post_action(&request, None, None).await;
        let primary_text = read_reply(primary_secret_name, reply)?;
        let mut secrets =
            parse_primary_secret(primary_text.as_ref()).map_err(AwsReadError::Primary)?;
        Ok(secrets.remove(secret_name))
    }
}
