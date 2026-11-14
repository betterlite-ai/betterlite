use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credential_sources::getters::VertexParams;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

use super::error::VertexRerankError;

// both LiteLLM entry points read credentials then project from copies of the params and authenticate on their own
async fn authenticate<'text, Lookup, Make>(
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
pub async fn complete_url<'text, Lookup, Make>(
    params: VertexParams<'text>,
    vertex: &'text VertexSettings,
    make_lookup: Make,
) -> Result<String, VertexRerankError>
where
    Make: FnOnce(Option<&ConfiguredCredentials>, Option<&str>) -> Lookup,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let access = authenticate(params, vertex, make_lookup).await?;
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

pub async fn request_headers<'text, Lookup, Make>(
    params: VertexParams<'text>,
    vertex: &'text VertexSettings,
    caller_headers: &BTreeMap<String, String>,
    make_lookup: Make,
) -> Result<BTreeMap<String, SecretString>, VertexRerankError>
where
    Make: FnOnce(Option<&ConfiguredCredentials>, Option<&str>) -> Lookup,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let access = authenticate(params, vertex, make_lookup).await?;
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
        SecretString::from(access.project),
    );
    for (name, value) in caller_headers {
        headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    Ok(headers)
}
