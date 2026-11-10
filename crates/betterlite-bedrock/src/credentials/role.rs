use std::borrow::Cow;
use std::time::Duration;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use chrono::{DateTime, Utc};
use secrecy::SecretString;

use crate::assume_role::is_already_running_as_role;
use crate::aws_settings::AwsSessionTag;
use crate::aws_settings::sources::{Credentials, NativeSources, ResolvedCredentials};

use super::AwsCredentialResolver;
use super::error::CredentialError;

pub mod aws_role;

use aws_role::auth_with_aws_role;

pub struct RoleAuth<'text> {
    pub role_name: &'text str,
    pub session_name: &'text str,
    pub access_key_id: Option<&'text SecretString>,
    pub secret_access_key: Option<&'text SecretString>,
    pub session_token: Option<&'text SecretString>,
    pub external_id: Option<&'text str>,
    pub irsa_role_arn: Option<&'text str>,
    pub web_identity_token_file: Option<&'text str>,
    pub tags: Option<&'text [AwsSessionTag]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CredentialTtl {
    Fractional(f64),
    Whole(i64),
}

#[derive(Debug)]
pub struct RoleCredentials {
    pub credentials: Credentials,
    pub ttl: Option<CredentialTtl>,
}

pub(super) async fn resolve_role_credentials(
    resolver: &AwsCredentialResolver<'_>,
    settings: &AwsSettings,
    role_name: &str,
    tags: Option<&[AwsSessionTag]>,
) -> Result<ResolvedCredentials, CredentialError> {
    if is_already_running_as_role(role_name, resolver.native).await {
        tracing::debug!("Already running as target role {role_name}, using ambient credentials");
        let credentials = resolver.native.load(&AwsSettings::default()).await?;
        return Ok(ResolvedCredentials {
            credentials,
            cache_ttl: None,
        });
    }
    tracing::debug!("Using role assumption: calling _auth_with_aws_role");
    let session_name = session_name(settings, resolver.native);
    let auth = RoleAuth {
        role_name,
        session_name: &session_name,

        access_key_id: settings.access_key_id.as_ref(),
        secret_access_key: settings.secret_access_key.as_ref(),
        session_token: settings.session_token.as_ref(),
        external_id: settings.external_id.as_deref(),
        irsa_role_arn: settings.role_arn.as_deref(),
        web_identity_token_file: settings.web_identity_token_file.as_deref(),
        tags,
    };
    let role_credentials = auth_with_aws_role(resolver, &auth).await?;
    Ok(ResolvedCredentials {
        credentials: role_credentials.credentials,
        cache_ttl: cache_ttl(role_credentials.ttl),
    })
}

fn session_name<'text>(settings: &'text AwsSettings, native: &NativeSources) -> Cow<'text, str> {
    if let Some(name) = settings
        .session_name
        .as_deref()
        .filter(|name| !name.is_empty())
    {
        return Cow::Borrowed(name);
    }
    Cow::Owned(format!(
        "litellm-session-{}",
        DateTime::<Utc>::from(native.clock.now()).timestamp()
    ))
}

fn cache_ttl(ttl: Option<CredentialTtl>) -> Option<Duration> {
    match ttl {
        Some(CredentialTtl::Fractional(seconds)) => {
            Some(Duration::try_from_secs_f64(seconds).unwrap_or(Duration::ZERO))
        }
        Some(CredentialTtl::Whole(seconds)) => {
            Some(u64::try_from(seconds).map_or(Duration::ZERO, Duration::from_secs))
        }
        None => None,
    }
}
