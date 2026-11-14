use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_openai::api::vector_store_records::records::vector::{
    BaseVectorStoreAuthCredentials, VectorStoreIndexEndpoints,
};
use secrecy::ExposeSecret as _;

use super::error::VertexRagStoreError;
use crate::credential_sources::getters::VertexParams;
use crate::endpoint_target::headers::set_headers;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::model_urls::base_url::get_vertex_base_url;
use crate::model_urls::error::VertexUrlError;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::token_refresh::ensure_access_token;

#[cfg(test)]
mod tests;

// LiteLLM's get_vector_store_endpoints_by_type: read searches contexts and write creates a corpus
#[must_use]
pub fn endpoints() -> VectorStoreIndexEndpoints {
    VectorStoreIndexEndpoints {
        read: Some(vec![("POST".to_owned(), ":retrieveContexts".to_owned())]),
        write: Some(vec![("POST".to_owned(), "/ragCorpora".to_owned())]),
        extra: BTreeMap::new(),
    }
}

pub struct Authorization<'request> {
    pub params: VertexParams<'request>,
    pub settings: &'request VertexSettings,
}

// LiteLLM's get_auth_credentials: a fresh Bearer token and JSON content type, never a caller header
pub async fn auth_credentials<Make, Lookup>(
    authorization: Authorization<'_>,
    make_lookup: Make,
) -> Result<BaseVectorStoreAuthCredentials, VertexRagStoreError>
where
    Make: FnOnce(Option<&ConfiguredCredentials>, Option<&str>) -> Lookup,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let credentials = authorization
        .params
        .safe_get_vertex_ai_credentials(authorization.settings);
    let project = authorization
        .params
        .safe_get_vertex_ai_project(authorization.settings);
    let access = ensure_access_token(
        VertexCapabilityProvider::VertexAi,
        make_lookup(credentials, project),
    )
    .await
    .map_err(VertexRagStoreError::Credentials)?;
    let mut headers = BTreeMap::new();
    for (name, value) in set_headers(Some(&access.token), &BTreeMap::new()) {
        headers.insert(name, serde_json::json!(value.expose_secret()));
    }
    Ok(BaseVectorStoreAuthCredentials {
        headers: Some(headers),
        query_params: None,
        extra: BTreeMap::new(),
    })
}

// LiteLLM's validate_environment: the auth headers overwrite the caller's Authorization and Content-Type
pub fn validate_environment(
    mut headers: BTreeMap<String, String>,
    credentials: &BaseVectorStoreAuthCredentials,
) -> Result<BTreeMap<String, String>, VertexRagStoreError> {
    let Some(auth) = &credentials.headers else {
        return Ok(headers);
    };
    for (name, value) in auth {
        let Some(text) = value.as_str() else {
            return Err(VertexRagStoreError::HeaderText(name.clone()));
        };
        headers.insert(name.clone(), text.to_owned());
    }
    Ok(headers)
}

// LiteLLM's get_complete_url: a truthy api_base is only trimmed, and the region host comes from B1305
pub fn get_complete_url(
    api_base: Option<&str>,
    params: VertexParams<'_>,
    settings: &VertexSettings,
) -> Result<String, VertexRagStoreError> {
    if let Some(base) = api_base.filter(|base| !base.is_empty()) {
        return Ok(base.trim_end_matches('/').to_owned());
    }
    let location = params
        .safe_get_vertex_ai_location(settings)
        .ok_or(VertexUrlError::LocationRequired)?;
    let project = params
        .safe_get_vertex_ai_project(settings)
        .ok_or(VertexUrlError::ProjectRequired)?;
    let base = get_vertex_base_url(Some(location))?;
    Ok(format!("{base}/v1/projects/{project}/locations/{location}"))
}
