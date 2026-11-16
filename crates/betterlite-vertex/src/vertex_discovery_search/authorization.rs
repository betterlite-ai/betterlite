use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_http::url_path::encode_segment;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    BaseVectorStoreAuthCredentials, VectorStoreIndexEndpoints,
};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::VertexDiscoverySearchError;
use super::options::VertexSearchOptions;
use crate::credential_sources::getters::VertexParams;
use crate::endpoint_target::headers::set_headers;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::model_urls::error::VertexUrlError;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::token_refresh::ensure_access_token;

mod tests;

const DISCOVERY_ENGINE_BASE: &str = "https://discoveryengine.googleapis.com/v1";
const COLLECTION_FIELD: &str = "vertex_collection_id";
const ENGINE_FIELD: &str = "vertex_engine_id";
const STORE_FIELD: &str = "vector_store_id";

// LiteLLM's get_vector_store_endpoints_by_type: a search read and no write endpoint
#[must_use]
pub fn endpoints() -> VectorStoreIndexEndpoints {
    VectorStoreIndexEndpoints {
        read: Some(vec![("POST".to_owned(), ":search".to_owned())]),
        write: Some(Vec::new()),
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
) -> Result<BaseVectorStoreAuthCredentials, VertexDiscoverySearchError>
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
    .map_err(VertexDiscoverySearchError::Credentials)?;
    Ok(BaseVectorStoreAuthCredentials {
        headers: Some(bearer_headers(&access.token)),
        query_params: None,
        extra: BTreeMap::new(),
    })
}

fn bearer_headers(token: &SecretString) -> BTreeMap<String, Json> {
    set_headers(Some(token), &BTreeMap::new())
        .into_iter()
        .map(|(name, value)| (name, Json::Text(value.expose_secret().to_owned())))
        .collect()
}

// LiteLLM's validate_environment: the auth headers overwrite the caller's Authorization and Content-Type
pub fn validate_environment(
    headers: BTreeMap<String, String>,
    credentials: &BaseVectorStoreAuthCredentials,
) -> Result<BTreeMap<String, String>, VertexDiscoverySearchError> {
    let Some(auth) = &credentials.headers else {
        return Ok(headers);
    };
    let overrides = auth
        .iter()
        .map(|(name, value)| {
            value
                .as_str()
                .map(|text| (name.clone(), text.to_owned()))
                .ok_or_else(|| VertexDiscoverySearchError::HeaderText(name.clone()))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(headers.into_iter().chain(overrides).collect())
}

// LiteLLM's get_complete_url: an api_base is only trimmed, and the host is fixed to Discovery Engine
pub fn get_complete_url(
    api_base: Option<&str>,
    params: VertexParams<'_>,
    settings: &VertexSettings,
    options: &VertexSearchOptions,
) -> Result<String, VertexDiscoverySearchError> {
    if let Some(base) = api_base.filter(|base| !base.is_empty()) {
        return Ok(base.trim_end_matches('/').to_owned());
    }
    let location = params
        .safe_get_vertex_ai_location(settings)
        .ok_or(VertexUrlError::LocationRequired)?;
    let project = params
        .safe_get_vertex_ai_project(settings)
        .ok_or(VertexUrlError::ProjectRequired)?;
    let collection = encode_segment(&options.vertex_collection_id, COLLECTION_FIELD)
        .map_err(|source| VertexDiscoverySearchError::Segment(COLLECTION_FIELD, source))?;
    let base = format!(
        "{DISCOVERY_ENGINE_BASE}/projects/{project}/locations/{location}/collections/{collection}"
    );
    let Some(engine) = options.vertex_engine_id.as_deref() else {
        let store = options
            .vector_store_id
            .as_deref()
            .ok_or(VertexDiscoverySearchError::StoreRequired)?;
        let store = encode_segment(store, STORE_FIELD)
            .map_err(|source| VertexDiscoverySearchError::Segment(STORE_FIELD, source))?;
        return Ok(format!(
            "{base}/dataStores/{store}/servingConfigs/default_config"
        ));
    };
    let engine = encode_segment(engine, ENGINE_FIELD)
        .map_err(|source| VertexDiscoverySearchError::Segment(ENGINE_FIELD, source))?;
    Ok(format!(
        "{base}/engines/{engine}/servingConfigs/default_serving_config"
    ))
}
