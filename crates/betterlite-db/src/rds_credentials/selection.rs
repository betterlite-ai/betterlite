use aws_credential_types::Credentials;
use betterlite_secret_managers::secret_lookup::SecretValue;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::RdsCredentialsError;
use super::secrets::RdsSecrets;
use super::sts::{
    ambient_credentials, assume_role, assume_role_with_web_identity, profile_credentials,
};
use super::{CREDENTIAL_PROVIDER, RdsCredentialInputs, RdsSources, RdsTimeout};

pub(super) struct Selection {
    pub(super) inputs: RdsCredentialInputs,
    pub(super) region: String,
    pub(super) timeout: RdsTimeout,
}

pub(super) fn prepare(
    inputs: RdsCredentialInputs,
    secrets: &dyn RdsSecrets,
) -> Result<Selection, RdsCredentialsError> {
    // LiteLLM reads both region names before it resolves any os.environ/ parameter reference
    let custom_region = secrets
        .get_secret("AWS_REGION_NAME")
        .map_err(|source| RdsCredentialsError::Secret { source })?;
    let standard_region = secrets
        .get_secret("AWS_REGION")
        .map_err(|source| RdsCredentialsError::Secret { source })?;
    let timeout = inputs.timeout;
    let resolved = resolve_references(inputs, secrets)?;
    let region = select_region(&resolved, custom_region, standard_region)?;
    Ok(Selection {
        inputs: resolved,
        region,
        timeout,
    })
}

pub(super) async fn select(
    selection: &Selection,
    sources: &RdsSources<'_>,
) -> Result<Credentials, RdsCredentialsError> {
    let inputs = &selection.inputs;
    let region = selection.region.as_str();
    if let (Some(token), Some(role), Some(session)) = (
        &inputs.web_identity_token,
        &inputs.role_name,
        &inputs.session_name,
    ) {
        let oidc_token = read_oidc_token(token, sources).await?;
        return assume_role_with_web_identity(role, session, &oidc_token, region, sources).await;
    }
    if let (Some(role), Some(session)) = (&inputs.role_name, &inputs.session_name) {
        return assume_role(
            role,
            session,
            explicit_credentials(inputs)?,
            region,
            sources,
        )
        .await;
    }
    if let Some(access) = &inputs.access_key_id {
        let secret =
            inputs
                .secret_access_key
                .as_ref()
                .ok_or(RdsCredentialsError::PartialCredentials {
                    missing: "aws_secret_access_key",
                })?;
        return Ok(Credentials::new(
            access.expose_secret(),
            secret.expose_secret(),
            None,
            None,
            CREDENTIAL_PROVIDER,
        ));
    }
    if let Some(profile) = &inputs.profile_name {
        return profile_credentials(profile, region, sources).await;
    }
    ambient_credentials(region, sources).await
}

fn resolve_references(
    inputs: RdsCredentialInputs,
    secrets: &dyn RdsSecrets,
) -> Result<RdsCredentialInputs, RdsCredentialsError> {
    Ok(RdsCredentialInputs {
        access_key_id: resolve_secret_reference(inputs.access_key_id, secrets)?,
        secret_access_key: resolve_secret_reference(inputs.secret_access_key, secrets)?,
        region_name: resolve_text_reference(inputs.region_name, secrets)?,
        session_name: resolve_text_reference(inputs.session_name, secrets)?,
        profile_name: resolve_text_reference(inputs.profile_name, secrets)?,
        role_name: resolve_text_reference(inputs.role_name, secrets)?,
        web_identity_token: resolve_secret_reference(inputs.web_identity_token, secrets)?,
        timeout: inputs.timeout,
    })
}

fn resolve_secret_reference(
    value: Option<SecretString>,
    secrets: &dyn RdsSecrets,
) -> Result<Option<SecretString>, RdsCredentialsError> {
    let Some(text) = value else {
        return Ok(None);
    };
    let Some(name) = text.expose_secret().strip_prefix("os.environ/") else {
        return Ok(Some(text));
    };
    let resolved = secrets
        .get_secret(name)
        .map_err(|source| RdsCredentialsError::Secret { source })?;
    Ok(secret_text(resolved)?.map(SecretString::from))
}

fn resolve_text_reference(
    value: Option<String>,
    secrets: &dyn RdsSecrets,
) -> Result<Option<String>, RdsCredentialsError> {
    let Some(text) = value else {
        return Ok(None);
    };
    let Some(name) = text.strip_prefix("os.environ/") else {
        return Ok(Some(text));
    };
    let resolved = secrets
        .get_secret(name)
        .map_err(|source| RdsCredentialsError::Secret { source })?;
    secret_text(resolved)
}

fn secret_text(value: Option<SecretValue>) -> Result<Option<String>, RdsCredentialsError> {
    match value {
        None => Ok(None),
        Some(SecretValue::Text(text)) => Ok(Some(text.expose_secret().to_owned())),
        Some(SecretValue::Bool(_)) => Err(RdsCredentialsError::SecretNotText),
    }
}

fn select_region(
    inputs: &RdsCredentialInputs,
    custom_region: Option<SecretValue>,
    standard_region: Option<SecretValue>,
) -> Result<String, RdsCredentialsError> {
    let truthy = |value: &str| !value.is_empty();
    if let Some(region) = inputs.region_name.as_deref().filter(|value| truthy(value)) {
        return Ok(region.to_owned());
    }
    if let Some(region) = secret_text(custom_region)?.filter(|value| truthy(value)) {
        return Ok(region);
    }
    if let Some(region) = secret_text(standard_region)?.filter(|value| truthy(value)) {
        return Ok(region);
    }
    Err(RdsCredentialsError::MissingRegion)
}

fn explicit_credentials(
    inputs: &RdsCredentialInputs,
) -> Result<Option<Credentials>, RdsCredentialsError> {
    match (&inputs.access_key_id, &inputs.secret_access_key) {
        (None, None) => Ok(None),
        (Some(access), Some(secret)) => Ok(Some(Credentials::new(
            access.expose_secret(),
            secret.expose_secret(),
            None,
            None,
            CREDENTIAL_PROVIDER,
        ))),
        (Some(_), None) => Err(RdsCredentialsError::PartialCredentials {
            missing: "aws_secret_access_key",
        }),
        (None, Some(_)) => Err(RdsCredentialsError::PartialCredentials {
            missing: "aws_access_key_id",
        }),
    }
}

async fn read_oidc_token(
    token: &SecretString,
    sources: &RdsSources<'_>,
) -> Result<SecretString, RdsCredentialsError> {
    let path = token.expose_secret();
    if let Ok(bytes) = sources.filesystem.read_to_end(path).await
        && let Ok(text) = String::from_utf8(bytes)
    {
        return Ok(SecretString::from(text));
    }
    let resolved = sources
        .secrets
        .get_secret(path)
        .map_err(|source| RdsCredentialsError::Secret { source })?;
    secret_text(resolved)?
        .map(SecretString::from)
        .ok_or(RdsCredentialsError::MissingOidcToken)
}
