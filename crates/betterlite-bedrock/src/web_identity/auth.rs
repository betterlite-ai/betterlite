use super::claims::decode_web_identity_claims;
use super::error::WebIdentityError;
use super::sts::{required_credentials, sts_client};
use crate::aws_endpoints::sts::StsClientKwargs;
use crate::aws_settings::error::{AWS_CONSOLE_URL, AwsAuthError};
use crate::aws_settings::policy::web_identity_policy;
use crate::aws_settings::sources::{Credentials, NativeSources};
use aws_sdk_sts::Client;
use aws_sdk_sts::error::SdkError;
use aws_sdk_sts::operation::assume_role_with_web_identity::AssumeRoleWithWebIdentityError;
use aws_sdk_sts::types::Credentials as ResponseCredentials;
use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use betterlite_secret_managers::secret_lookup::{SecretLookup, SecretValue};
use secrecy::{ExposeSecret as _, SecretString};
use std::time::Duration;
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
    let (status, message) = if auth.token_reference.starts_with("os.environ/")
        || native.environment.contains_key(auth.token_reference)
    {
        (400, "Invalid web identity token reference.")
    } else {
        match secrets.get_secret(&native.environment, auth.token_reference, None)? {
            Some(SecretValue::Text(token)) => return Ok(token),
            Some(SecretValue::Bool(_)) => return Err(WebIdentityError::SecretType),
            None => (
                401,
                "OIDC token could not be retrieved from secret manager.",
            ),
        }
    };
    let request = ErrorRequest {
        method: "POST".to_owned(),
        url: AWS_CONSOLE_URL.to_owned(),
    };
    Err(WebIdentityError::Auth(Box::new(AwsAuthError {
        status,
        message: message.to_owned(),
        response: ErrorResponse {
            status_code: status,
            request: request.clone(),
        },
        request,
    })))
}
#[expect(
    deprecated,
    reason = "LiteLLM reads PackedPolicySize; aws-sdk-sts 1.116.0 deprecates it"
)]
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
    let credentials = required_credentials(response.credentials(), "web identity")?;
    let packed_policy_size = response.packed_policy_size().ok_or_else(|| {
        WebIdentityError::MissingField(
            "the STS web identity response has no PackedPolicySize".to_owned(),
        )
    })?;
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
            access_key_id: SecretString::from(response_credentials.access_key_id()),
            secret_access_key: SecretString::from(response_credentials.secret_access_key()),
            session_token: Some(SecretString::from(response_credentials.session_token())),
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
        let message = format!("AWS STS rejected the web identity token: {error}.{detail}");
        let request = ErrorRequest {
            method: "POST".to_owned(),
            url: AWS_CONSOLE_URL.to_owned(),
        };
        return WebIdentityError::RejectedToken {
            error: Box::new(AwsAuthError {
                status: 401,
                message,
                response: ErrorResponse {
                    status_code: 401,
                    request: request.clone(),
                },
                request,
            }),
            source: Box::new(error),
        };
    }
    WebIdentityError::WebIdentity {
        source: Box::new(error),
    }
}
