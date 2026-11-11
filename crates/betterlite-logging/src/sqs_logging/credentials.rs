use std::sync::Arc;

use async_trait::async_trait;
use aws_credential_types::Credentials;
use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};

pub const ACCESS_KEY_VARIABLE: &str = "AWS_ACCESS_KEY_ID";
pub const SECRET_ACCESS_KEY_VARIABLE: &str = "AWS_SECRET_ACCESS_KEY";
pub const SESSION_TOKEN_VARIABLE: &str = "AWS_SESSION_TOKEN";
pub const REGION_VARIABLE: &str = "AWS_REGION_NAME";
pub const SESSION_NAME_VARIABLE: &str = "AWS_SESSION_NAME";
pub const PROFILE_NAME_VARIABLE: &str = "AWS_PROFILE_NAME";
pub const ROLE_NAME_VARIABLE: &str = "AWS_ROLE_NAME";
pub const WEB_IDENTITY_TOKEN_VARIABLE: &str = "AWS_WEB_IDENTITY_TOKEN";
pub const STS_ENDPOINT_VARIABLE: &str = "AWS_STS_ENDPOINT";

const PROVIDER_NAME: &str = "betterlite-sqs";

#[derive(Clone, Debug, Default)]
pub struct CredentialParams {
    pub access_key_id: Option<SecretString>,
    pub secret_access_key: Option<SecretString>,
    pub session_token: Option<SecretString>,
    pub region_name: Option<String>,
    pub session_name: Option<String>,
    pub profile_name: Option<String>,
    pub role_name: Option<String>,
    pub web_identity_token: Option<SecretString>,
    pub sts_endpoint: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error(
        "the SQS credential setting '{name}' is not supported; set sqs_aws_access_key_id and sqs_aws_secret_access_key instead"
    )]
    Unsupported { name: &'static str },
    #[error(
        "no AWS credentials were found for the SQS logger; set sqs_aws_access_key_id and sqs_aws_secret_access_key, or AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY"
    )]
    Missing,
}

#[async_trait]
pub trait CredentialSource: Send + Sync {
    fn validate(&self, params: &CredentialParams) -> Result<(), CredentialError>;

    async fn resolve(&self, params: &CredentialParams) -> Result<Credentials, CredentialError>;
}

pub struct EnvironmentCredentials {
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
}

fn text_or_environment(
    explicit: Option<&String>,
    environment: &dyn EnvironmentLookup,
    name: &str,
) -> Option<String> {
    if explicit.is_some() {
        explicit.cloned()
    } else {
        environment.lookup(name)
    }
}

fn secret_or_environment(
    explicit: Option<&SecretString>,
    environment: &dyn EnvironmentLookup,
    name: &str,
) -> Option<SecretString> {
    if explicit.is_some() {
        explicit.cloned()
    } else {
        environment.lookup(name).map(SecretString::from)
    }
}

fn effective(params: &CredentialParams, environment: &dyn EnvironmentLookup) -> CredentialParams {
    CredentialParams {
        access_key_id: secret_or_environment(
            params.access_key_id.as_ref(),
            environment,
            ACCESS_KEY_VARIABLE,
        ),
        secret_access_key: secret_or_environment(
            params.secret_access_key.as_ref(),
            environment,
            SECRET_ACCESS_KEY_VARIABLE,
        ),
        session_token: secret_or_environment(
            params.session_token.as_ref(),
            environment,
            SESSION_TOKEN_VARIABLE,
        ),
        region_name: text_or_environment(params.region_name.as_ref(), environment, REGION_VARIABLE),
        session_name: text_or_environment(
            params.session_name.as_ref(),
            environment,
            SESSION_NAME_VARIABLE,
        ),
        profile_name: text_or_environment(
            params.profile_name.as_ref(),
            environment,
            PROFILE_NAME_VARIABLE,
        ),
        role_name: text_or_environment(params.role_name.as_ref(), environment, ROLE_NAME_VARIABLE),
        web_identity_token: secret_or_environment(
            params.web_identity_token.as_ref(),
            environment,
            WEB_IDENTITY_TOKEN_VARIABLE,
        ),
        sts_endpoint: text_or_environment(
            params.sts_endpoint.as_ref(),
            environment,
            STS_ENDPOINT_VARIABLE,
        ),
    }
}

fn validate_params(params: &CredentialParams) -> Result<(), CredentialError> {
    let unsupported = [
        ("sqs_aws_profile_name", params.profile_name.is_some()),
        ("sqs_aws_role_name", params.role_name.is_some()),
        (
            "sqs_aws_web_identity_token",
            params.web_identity_token.is_some(),
        ),
        ("sqs_aws_sts_endpoint", params.sts_endpoint.is_some()),
    ];
    match unsupported.into_iter().find(|(_name, present)| *present) {
        Some((name, _present)) => Err(CredentialError::Unsupported { name }),
        None => Ok(()),
    }
}

#[async_trait]
impl CredentialSource for EnvironmentCredentials {
    fn validate(&self, params: &CredentialParams) -> Result<(), CredentialError> {
        validate_params(&effective(params, self.environment.as_ref()))
    }

    async fn resolve(&self, params: &CredentialParams) -> Result<Credentials, CredentialError> {
        let effective_params = effective(params, self.environment.as_ref());
        validate_params(&effective_params)?;
        let (Some(access_key_id), Some(secret_access_key)) = (
            effective_params.access_key_id,
            effective_params.secret_access_key,
        ) else {
            return Err(CredentialError::Missing);
        };
        let session_token = effective_params
            .session_token
            .map(|token| token.expose_secret().to_owned());
        Ok(Credentials::new(
            access_key_id.expose_secret().to_owned(),
            secret_access_key.expose_secret().to_owned(),
            session_token,
            None,
            PROVIDER_NAME,
        ))
    }
}

mod tests;
