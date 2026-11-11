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

fn effective(params: &CredentialParams, environment: &dyn EnvironmentLookup) -> CredentialParams {
    let mut resolved = params.clone();
    for (slot, name) in [
        (&mut resolved.access_key_id, ACCESS_KEY_VARIABLE),
        (&mut resolved.secret_access_key, SECRET_ACCESS_KEY_VARIABLE),
        (&mut resolved.session_token, SESSION_TOKEN_VARIABLE),
        (
            &mut resolved.web_identity_token,
            WEB_IDENTITY_TOKEN_VARIABLE,
        ),
    ] {
        if slot.is_none() {
            *slot = environment.lookup(name).map(SecretString::from);
        }
    }
    for (slot, name) in [
        (&mut resolved.region_name, REGION_VARIABLE),
        (&mut resolved.session_name, SESSION_NAME_VARIABLE),
        (&mut resolved.profile_name, PROFILE_NAME_VARIABLE),
        (&mut resolved.role_name, ROLE_NAME_VARIABLE),
        (&mut resolved.sts_endpoint, STS_ENDPOINT_VARIABLE),
    ] {
        if slot.is_none() {
            *slot = environment.lookup(name);
        }
    }
    resolved
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
