use std::collections::HashMap;

use aws_config::{BehaviorVersion, Region, retry::RetryConfig, timeout::TimeoutConfig};
use aws_credential_types::Credentials as SdkCredentials;
use aws_sdk_sts::Client;
use aws_sdk_sts::config::Builder;
use aws_sdk_sts::error::SdkError;
use aws_sdk_sts::operation::assume_role::AssumeRoleError;
use aws_sdk_sts::types::Credentials as ResponseCredentials;
use aws_sdk_sts::types::Tag;
use aws_smithy_types::error::display::DisplayErrorContext;
use aws_smithy_types::error::metadata::ProvideErrorMetadata;
use aws_types::os_shim_internal::Env;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use chrono::{DateTime, TimeDelta, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use crate::assume_role::error::ExpiryError;
use crate::assume_role::expiry::{StsResponse, extract_credentials_and_ttl};
use crate::assume_role::is_already_running_as_role;
use crate::aws_endpoints::sts::StsClientKwargs;
use crate::aws_settings::sources::{Credentials, NativeSources};
use crate::aws_settings::{AwsSessionTag, assume_role_parameters};
use crate::web_identity::error::WebIdentityError;
use crate::web_identity::irsa::{
    CrossAccountIrsa, SameAccountIrsa, handle_irsa_cross_account, handle_irsa_same_account,
};

use crate::credentials::AwsCredentialResolver;
use crate::credentials::error::CredentialError;
use crate::credentials::role::{CredentialTtl, RoleAuth, RoleCredentials};

pub async fn auth_with_aws_role(
    resolver: &AwsCredentialResolver<'_>,
    auth: &RoleAuth<'_>,
) -> Result<RoleCredentials, CredentialError> {
    let token_file = auth
        .web_identity_token_file
        .filter(|value| !value.is_empty());
    let irsa_role = auth.irsa_role_arn.filter(|value| !value.is_empty());
    let no_explicit_keys = auth.access_key_id.is_none() && auth.secret_access_key.is_none();
    if let (Some(token_path), Some(irsa_arn)) = (token_file, irsa_role)
        && no_explicit_keys
    {
        return irsa_credentials(resolver, auth, irsa_arn, token_path).await;
    }
    assume_role_credentials(resolver, auth, no_explicit_keys).await
}

enum IrsaRequest<'text> {
    Same(SameAccountIrsa<'text>),
    Cross(CrossAccountIrsa<'text>),
}

async fn irsa_credentials(
    resolver: &AwsCredentialResolver<'_>,
    auth: &RoleAuth<'_>,
    irsa_role: &str,
    token_file: &str,
) -> Result<RoleCredentials, CredentialError> {
    tracing::debug!("IRSA detected: using web identity token from {token_file}");
    let request = if auth.role_name == irsa_role {
        IrsaRequest::Same(SameAccountIrsa {
            role_arn: auth.role_name,
            session_name: auth.session_name,
            external_id: auth.external_id,
            tags: auth.tags,
        })
    } else {
        IrsaRequest::Cross(CrossAccountIrsa {
            irsa_role_arn: irsa_role,
            role_arn: auth.role_name,
            session_name: auth.session_name,
            token_file,
            external_id: auth.external_id,
            tags: auth.tags,
        })
    };
    match irsa_assume(resolver, &request).await {
        Ok(response) => irsa_result(response, resolver),
        Err(error) => {
            tracing::debug!("Failed to assume role via IRSA: {error}");
            if irsa_trust_policy_denied(&error) {
                let role_name = auth.role_name;
                tracing::error!(
                    role = role_name,
                    "Access denied when trying to assume role {role_name}. Please ensure the trust policy of {role_name} allows the current role to assume it. Current identity: check logs with verbose mode."
                );
            }
            Err(CredentialError::WebIdentity(error))
        }
    }
}

fn irsa_result(
    response: StsResponse,
    resolver: &AwsCredentialResolver<'_>,
) -> Result<RoleCredentials, CredentialError> {
    let now = DateTime::<Utc>::from(resolver.native.clock.now());
    let (credentials, ttl) = extract_credentials_and_ttl(response, now)?;
    Ok(RoleCredentials {
        credentials,
        ttl: Some(CredentialTtl::Whole(ttl)),
    })
}

async fn irsa_assume(
    resolver: &AwsCredentialResolver<'_>,
    request: &IrsaRequest<'_>,
) -> Result<StsResponse, WebIdentityError> {
    match request {
        IrsaRequest::Same(input) => {
            handle_irsa_same_account(input, resolver.sts, resolver.native).await
        }
        IrsaRequest::Cross(input) => {
            handle_irsa_cross_account(input, resolver.sts, resolver.native).await
        }
    }
}

async fn assume_role_credentials(
    resolver: &AwsCredentialResolver<'_>,
    auth: &RoleAuth<'_>,
    no_explicit_keys: bool,
) -> Result<RoleCredentials, CredentialError> {
    let client_credentials = (!no_explicit_keys).then(|| explicit_credentials(auth));
    let client = sts_client(resolver.sts, resolver.native, client_credentials).await;
    let parameters = assume_role_parameters(
        auth.role_name.to_owned(),
        auth.session_name.to_owned(),
        auth.external_id.map(str::to_owned),
        auth.tags.map(<[AwsSessionTag]>::to_vec),
    );
    let tags = parameters.tags.as_deref().map(tags_to_sdk).transpose()?;
    let sent = client
        .assume_role()
        .role_arn(parameters.role_arn)
        .role_session_name(parameters.role_session_name)
        .set_external_id(parameters.external_id)
        .set_tags(tags)
        .send()
        .await;
    let response = match sent {
        Ok(response) => response,
        Err(error) => return denied_assume_role(resolver, auth, error).await,
    };
    let Some(sts) = response.credentials() else {
        return Err(CredentialError::MissingCredentials);
    };
    sts_credentials(sts, DateTime::<Utc>::from(resolver.native.clock.now()))
}

async fn denied_assume_role(
    resolver: &AwsCredentialResolver<'_>,
    auth: &RoleAuth<'_>,
    error: SdkError<AssumeRoleError>,
) -> Result<RoleCredentials, CredentialError> {
    let error_text = DisplayErrorContext(&error).to_string();
    if access_denied(&error) {
        if is_already_running_as_role(auth.role_name, resolver.native).await {
            let role_name = auth.role_name;
            tracing::warn!(
                role = role_name,
                "AssumeRole failed for {role_name} ({error_text}). Caller is already running as this role; falling back to ambient credentials."
            );
            let credentials = resolver.native.load(&AwsSettings::default()).await?;
            return Ok(RoleCredentials {
                credentials,
                ttl: None,
            });
        }
        let role_name = auth.role_name;
        tracing::error!(
            role = role_name,
            "AssumeRole AccessDenied for {role_name} and caller is NOT the same role. Re-raising. Error: {error_text}"
        );
    }
    Err(CredentialError::AssumeRole {
        source: Box::new(error),
    })
}

fn explicit_credentials(auth: &RoleAuth<'_>) -> SdkCredentials {
    SdkCredentials::new(
        auth.access_key_id
            .map_or_else(String::new, |key| key.expose_secret().to_owned()),
        auth.secret_access_key
            .map_or_else(String::new, |key| key.expose_secret().to_owned()),
        auth.session_token
            .map(|token| token.expose_secret().to_owned()),
        None,
        "bedrock-role",
    )
}

fn sts_credentials(
    sts: &ResponseCredentials,
    now: DateTime<Utc>,
) -> Result<RoleCredentials, CredentialError> {
    let expiration =
        DateTime::<Utc>::from_timestamp(sts.expiration().secs(), sts.expiration().subsec_nanos())
            .ok_or(CredentialError::Expiry(ExpiryError::OutOfRange))?;
    let remaining = expiration
        .signed_duration_since(now)
        .checked_sub(&TimeDelta::minutes(1))
        .ok_or(CredentialError::Expiry(ExpiryError::OutOfRange))?;
    Ok(RoleCredentials {
        credentials: Credentials {
            access_key_id: SecretString::from(sts.access_key_id().to_owned()),
            secret_access_key: SecretString::from(sts.secret_access_key().to_owned()),
            session_token: Some(SecretString::from(sts.session_token().to_owned())),
        },
        ttl: Some(CredentialTtl::Fractional(remaining.as_seconds_f64())),
    })
}

fn tags_to_sdk(tags: &[AwsSessionTag]) -> Result<Vec<Tag>, CredentialError> {
    tags.iter()
        .map(|tag| {
            Tag::builder()
                .key(tag.key.clone())
                .value(tag.value.clone())
                .build()
                .map_err(|source| CredentialError::Tag { source })
        })
        .collect()
}

fn access_denied(error: &SdkError<AssumeRoleError>) -> bool {
    error.as_service_error().is_some_and(|service| {
        service
            .code()
            .is_some_and(|code| code.contains("AccessDenied"))
            || service
                .message()
                .is_some_and(|message| message.contains("AccessDenied"))
    })
}

fn irsa_trust_policy_denied(error: &WebIdentityError) -> bool {
    if let WebIdentityError::WebIdentity { source } = error {
        return denied_details(source.as_ref());
    }
    if let WebIdentityError::AssumeRole { source } = error {
        return denied_details(source.as_ref());
    }
    false
}

fn denied_details<E>(error: &SdkError<E>) -> bool
where
    E: ProvideErrorMetadata,
{
    let Some(service) = error.as_service_error() else {
        return false;
    };
    let text = format!(
        "{} {}",
        service.code().unwrap_or_default(),
        service.message().unwrap_or_default()
    );
    text.contains("AccessDenied") && text.contains("is not authorized to perform: sts:AssumeRole")
}

async fn sts_client(
    kwargs: &StsClientKwargs,
    native: &NativeSources,
    credentials: Option<SdkCredentials>,
) -> Client {
    let mut loader = aws_config::defaults(BehaviorVersion::latest())
        .env(Env::from(HashMap::from_iter(native.environment.clone())))
        .fs(native.filesystem.clone())
        .http_client(native.client.clone())
        .time_source(native.clock.clone())
        .sleep_impl(native.sleep.clone())
        .retry_config(RetryConfig::disabled())
        .timeout_config(
            TimeoutConfig::builder()
                .operation_timeout(native.timeout)
                .build(),
        );
    if let Some(provider) = credentials {
        loader = loader.credentials_provider(provider);
    }
    let shared = match kwargs.region_name.as_deref() {
        Some(region) => loader.region(Region::new(region.to_owned())).load().await,
        None => loader.load().await,
    };
    let mut builder = Builder::from(&shared);
    if let Some(endpoint) = &kwargs.endpoint_url {
        builder = builder.endpoint_url(endpoint.clone());
    }
    // boto3.client("sts") without any region talks to the aws-global endpoint
    if shared.region().is_none() {
        builder = builder.region(Region::from_static("aws-global"));
    }
    Client::from_conf(builder.build())
}
