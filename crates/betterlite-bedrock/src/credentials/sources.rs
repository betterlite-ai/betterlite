use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::tls::VerifySetting;
use secrecy::ExposeSecret as _;

use crate::aws_settings::AwsSessionTag;
use crate::aws_settings::sources::{
    Credentials, ResolvedCredentials, auth_selector_presence, resolve_local_credentials,
};
use crate::web_identity::auth::{WebIdentityAuth, auth_with_web_identity_token};

use super::AwsCredentialResolver;
use super::cache::{cached, parameters};
use super::error::CredentialError;
use super::role;

pub(super) async fn resolve(
    resolver: &AwsCredentialResolver<'_>,
    settings: &AwsSettings,
    region: Option<&str>,
    tags: Option<Vec<AwsSessionTag>>,
    ssl_verify: Option<&VerifySetting>,
) -> Result<Credentials, CredentialError> {
    let mut effective = settings.clone();
    effective.region_name = region.map(str::to_owned);
    tracing::debug!(
        access_key_id = settings.access_key_id.is_some(),
        secret_access_key = settings.secret_access_key.is_some(),
        session_token = settings.session_token.is_some(),
        "in get credentials"
    );
    let parameters = parameters(&effective, tags.as_deref(), ssl_verify);
    let [web_identity_selected, role, profile, session_token, _static] =
        auth_selector_presence(&effective);
    if web_identity_selected {
        return cached(resolver.cache, &parameters, || {
            web_identity(resolver, settings)
        })
        .await;
    }
    if role {
        let canonical_tags = tags.as_deref();
        return cached(resolver.cache, &parameters, || {
            role::resolve_role_credentials(resolver, &effective, canonical_tags)
        })
        .await;
    }
    if profile || session_token {
        let local = resolve_local_credentials(&effective, resolver.native).await?;
        return Ok(local.credentials);
    }
    cached(resolver.cache, &parameters, || async {
        resolve_local_credentials(&effective, resolver.native)
            .await
            .map_err(CredentialError::from)
    })
    .await
}

async fn web_identity(
    resolver: &AwsCredentialResolver<'_>,
    settings: &AwsSettings,
) -> Result<ResolvedCredentials, CredentialError> {
    let auth = WebIdentityAuth {
        token_reference: settings
            .web_identity_token
            .as_ref()
            .map_or("", |token| token.expose_secret()),
        role_arn: settings.role_name.as_deref().unwrap_or_default(),
        session_name: settings.session_name.as_deref().unwrap_or_default(),
        external_id: settings.external_id.as_deref(),
        max_policy_size: resolver.max_policy_size,
    };
    let web_identity_credentials =
        auth_with_web_identity_token(&auth, resolver.sts, resolver.secrets, resolver.native)
            .await?;
    Ok(ResolvedCredentials {
        credentials: web_identity_credentials.credentials,
        cache_ttl: Some(web_identity_credentials.cache_ttl),
    })
}
