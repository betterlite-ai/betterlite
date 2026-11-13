use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::AgentEngineError;
use crate::credential_sources::getters::VertexParams;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::token_refresh::ensure_access_token;

#[derive(Debug, Clone, Copy)]
pub struct AgentAuth<'call> {
    pub params: VertexParams<'call>,
    pub vertex: &'call VertexSettings,
}

pub async fn auth_headers<Lookup, Make>(
    auth: &AgentAuth<'_>,
    caller: &BTreeMap<String, String>,
    make_lookup: Make,
) -> Result<BTreeMap<String, SecretString>, AgentEngineError>
where
    Make: FnOnce(Option<&ConfiguredCredentials>, Option<&str>) -> Lookup,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let credentials = auth.params.safe_get_vertex_ai_credentials(auth.vertex);
    let project = auth.params.safe_get_vertex_ai_project(auth.vertex);
    let lookup = make_lookup(credentials, project);
    let access = ensure_access_token(VertexCapabilityProvider::VertexAi, lookup)
        .await
        .map_err(AgentEngineError::Auth)?;
    let mut headers: BTreeMap<String, SecretString> = caller
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect();
    // validate_environment updates the caller's headers, so these two replace whatever the caller sent
    let bearer = format!("Bearer {}", access.token.expose_secret());
    headers.insert("Authorization".to_owned(), SecretString::from(bearer));
    headers.insert(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    );
    Ok(headers)
}
mod tests;
