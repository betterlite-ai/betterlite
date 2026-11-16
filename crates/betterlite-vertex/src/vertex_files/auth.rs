use std::collections::BTreeMap;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use secrecy::{ExposeSecret as _, SecretString};

use crate::vertex_interactions::mint::MintAccessToken;

use super::error::VertexFilesError;

pub struct EnvironmentRequest<'request> {
    pub headers: BTreeMap<String, String>,
    pub api_key: Option<&'request SecretString>,
    pub credentials: Option<&'request ConfiguredCredentials>,
    pub project: Option<&'request str>,
}

// only a caller-supplied key leaves the headers alone; otherwise the credential cache mints the bearer token
pub async fn validate_environment(
    request: EnvironmentRequest<'_>,
    mint: &dyn MintAccessToken,
) -> Result<BTreeMap<String, SecretString>, VertexFilesError> {
    let mut headers: BTreeMap<String, SecretString> = request
        .headers
        .into_iter()
        .map(|(name, value)| (name, SecretString::from(value)))
        .collect();
    if request
        .api_key
        .is_some_and(|key| !key.expose_secret().is_empty())
    {
        return Ok(headers);
    }
    let access = mint.mint(request.credentials, request.project).await?;
    let token = access.token.expose_secret();
    if token.is_empty() {
        return Err(VertexFilesError::MissingApiKey);
    }
    headers.insert(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {token}")),
    );
    Ok(headers)
}

mod tests;
