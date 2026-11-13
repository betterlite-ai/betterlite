use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::tls::VerifySetting;
use secrecy::{ExposeSecret as _, SecretString};

use crate::aws_settings::canonicalize_tags;
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
    ssl_verify: Option<&VerifySetting>,
) -> Result<Credentials, CredentialError> {
    let tags = canonicalize_tags(settings.session_tags.clone())?;
    let mut effective = settings.clone();
    effective.region_name = region.map(str::to_owned);
    let parameters = parameters(&effective, tags.as_deref(), ssl_verify);
    let web_identity_inputs = (
        settings.web_identity_token.as_ref(),
        settings.role_name.as_deref(),
        settings.session_name.as_deref(),
    );
    if let (Some(token), Some(role_name), Some(session_name)) = web_identity_inputs {
        return cached(resolver.cache, &parameters, || {
            web_identity(resolver, settings, token, role_name, session_name)
        })
        .await;
    }
    if let Some(role_name) = settings.role_name.as_deref() {
        let canonical_tags = tags.as_deref();
        return cached(resolver.cache, &parameters, || {
            role::resolve_role_credentials(resolver, &effective, role_name, canonical_tags)
        })
        .await;
    }
    let [_web_identity, _role, profile, session_token, _static] =
        auth_selector_presence(&effective);
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
    token: &SecretString,
    role_name: &str,
    session_name: &str,
) -> Result<ResolvedCredentials, CredentialError> {
    let auth = WebIdentityAuth {
        token_reference: token.expose_secret(),
        role_arn: role_name,
        session_name,
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
