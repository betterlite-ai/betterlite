use std::time::Duration;

use aws_sdk_sts::Client;
use aws_sdk_sts::error::SdkError;
use aws_sdk_sts::operation::assume_role_with_web_identity::AssumeRoleWithWebIdentityError;
use aws_sdk_sts::types::Credentials as ResponseCredentials;
use betterlite_secret_managers::secret_lookup::{SecretLookup, SecretValue};
use secrecy::{ExposeSecret as _, SecretString};

use super::claims::decode_web_identity_claims;
use super::error::{WebIdentityError, auth_error};
use super::sts::sts_client;
use crate::aws_endpoints::sts::StsClientKwargs;
use crate::aws_settings::policy::web_identity_policy;
use crate::aws_settings::sources::{Credentials, NativeSources};

pub struct WebIdentityAuth<'text> {
    pub token_reference: &'text str,
    pub role_arn: &'text str,
    pub session_name: &'text str,
    pub external_id: Option<&'text str>,
    pub max_policy_size: i64,
}

#[derive(Debug)]
pub struct WebIdentityCredentials {
    pub credentials: Credentials,
    pub cache_ttl: Duration,
}

pub async fn auth_with_web_identity_token(
    auth: &WebIdentityAuth<'_>,
    sts: &StsClientKwargs,
    secrets: &SecretLookup,
    native: &NativeSources,
) -> Result<WebIdentityCredentials, WebIdentityError> {
    tracing::debug!(
        "IN Web Identity Token: [set] | Role Name: {} | Session Name: {}",
        auth.role_arn,
        auth.session_name
    );
    let token = resolve_token(auth, secrets, native)?;
    // AssumeRoleWithWebIdentity has no ExternalId member, so boto3 rejects the parameter
    if auth.external_id.is_some() {
        return Err(WebIdentityError::UnsupportedParameter {
            field: "aws_external_id",
        });
    }
    let client = sts_client(sts, native, None).await;
    let (response_credentials, packed_policy_size) = send(&client, auth, &token).await?;
    Ok(finish(auth, &response_credentials, packed_policy_size))
}

fn resolve_token(
    auth: &WebIdentityAuth<'_>,
    secrets: &SecretLookup,
    native: &NativeSources,
) -> Result<SecretString, WebIdentityError> {
    if auth.token_reference.starts_with("os.environ/")
        || native.environment.contains_key(auth.token_reference)
    {
        return Err(WebIdentityError::Auth(Box::new(auth_error(
            400,
            "Invalid web identity token reference.".to_owned(),
        ))));
    }
    match secrets.get_secret(&native.environment, auth.token_reference, None)? {
        Some(SecretValue::Text(token)) => Ok(token),
        Some(SecretValue::Bool(_)) => Err(WebIdentityError::SecretType {
            field: "aws_web_identity_token",
        }),
        None => Err(WebIdentityError::Auth(Box::new(auth_error(
            401,
            "OIDC token could not be retrieved from secret manager.".to_owned(),
        )))),
    }
}

async fn send(
    client: &Client,
    auth: &WebIdentityAuth<'_>,
    token: &SecretString,
) -> Result<(ResponseCredentials, i32), WebIdentityError> {
    let policy = serde_json::to_string(&web_identity_policy())
        .map_err(|source| WebIdentityError::Policy { source })?;
    let response = client
        .assume_role_with_web_identity()
        .role_arn(auth.role_arn)
        .role_session_name(auth.session_name)
        .web_identity_token(token.expose_secret())
        .duration_seconds(3_600)
        .policy(policy)
        .send()
        .await
        .map_err(|error| rejected(error, token.expose_secret()))?;
    let Some(credentials) = response.credentials() else {
        return Err(WebIdentityError::MissingField {
            call: "web identity",
            field: "Credentials",
        });
    };
    let Some(packed_policy_size) = response.packed_policy_size() else {
        return Err(WebIdentityError::MissingField {
            call: "web identity",
            field: "PackedPolicySize",
        });
    };
    Ok((credentials.clone(), packed_policy_size))
}

fn finish(
    auth: &WebIdentityAuth<'_>,
    response_credentials: &ResponseCredentials,
    packed_policy_size: i32,
) -> WebIdentityCredentials {
    if i64::from(packed_policy_size) > auth.max_policy_size {
        tracing::warn!(
            "The policy size is greater than 75% of the allowed size, PackedPolicySize: {packed_policy_size}"
        );
    }
    WebIdentityCredentials {
        credentials: Credentials {
            access_key_id: SecretString::from(response_credentials.access_key_id().to_owned()),
            secret_access_key: SecretString::from(
                response_credentials.secret_access_key().to_owned(),
            ),
            session_token: Some(SecretString::from(
                response_credentials.session_token().to_owned(),
            )),
        },
        cache_ttl: Duration::from_mins(59),
    }
}

fn rejected(error: SdkError<AssumeRoleWithWebIdentityError>, token: &str) -> WebIdentityError {
    if matches!(
        error.as_service_error(),
        Some(AssumeRoleWithWebIdentityError::InvalidIdentityTokenException(_))
    ) {
        let detail = decode_web_identity_claims(token)
            .map_or_else(String::new, |claims| format!(" Token {claims}"));
        return WebIdentityError::RejectedToken {
            error: Box::new(auth_error(
                401,
                format!("AWS STS rejected the web identity token: {error}.{detail}"),
            )),
            source: Box::new(error),
        };
    }
    WebIdentityError::WebIdentity {
        source: Box::new(error),
    }
}
