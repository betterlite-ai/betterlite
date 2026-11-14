use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credential_sources::getters::VertexParams;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

use super::error::VertexRerankError;

// get_complete_url and validate_environment each authenticate on their own, so a caller authenticates once for each
pub async fn authenticate<'text, Lookup, Make>(
    params: VertexParams<'text>,
    vertex: &'text VertexSettings,
    make_lookup: Make,
) -> Result<AccessToken, VertexRerankError>
where
    Make: FnOnce(Option<&ConfiguredCredentials>, Option<&str>) -> Lookup,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let credentials = params.safe_get_vertex_ai_credentials(vertex);
    let project = params.safe_get_vertex_ai_project(vertex);
    make_lookup(credentials, project)
        .await
        .map_err(VertexRerankError::Credentials)
}

// get_complete_url ignores api_base and the model: the ranking config is always the global default
pub fn complete_url(
    access: &AccessToken,
    vertex: &VertexSettings,
) -> Result<String, VertexRerankError> {
    // the loaded setting already folds VERTEXAI_PROJECT and the module project
    let configured = vertex.project.as_deref().unwrap_or_default();
    let project = [access.project.as_str(), configured]
        .into_iter()
        .find(|project| !project.is_empty())
        .ok_or(VertexRerankError::MissingProject)?;
    Ok(format!(
        "https://discoveryengine.googleapis.com/v1/projects/{project}/locations/global/rankingConfigs/default_ranking_config:rank"
    ))
}

#[must_use]
pub fn request_headers(
    access: &AccessToken,
    caller_headers: &BTreeMap<String, String>,
) -> BTreeMap<String, SecretString> {
    let mut headers = BTreeMap::new();
    headers.insert(
        String::from("Authorization"),
        SecretString::from(format!("Bearer {}", access.token.expose_secret())),
    );
    headers.insert(
        String::from("Content-Type"),
        SecretString::from("application/json"),
    );
    headers.insert(
        String::from("X-Goog-User-Project"),
        SecretString::from(access.project.clone()),
    );
    for (name, value) in caller_headers {
        headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    headers
}
