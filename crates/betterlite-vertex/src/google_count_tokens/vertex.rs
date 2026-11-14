use std::{collections::BTreeMap, time::Duration};

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_models::{catalog::ModelCatalog, decisions::Json};
use secrecy::{ExposeSecret as _, SecretString};

use crate::credential_sources::getters::VertexParams;
use crate::endpoint_target::target::{TargetProvider, TokenUrlInput, get_token_and_url};

use crate::google_count_tokens::transport::{
    CountRequest, CountTransport, count, count_body, header_map,
};
use crate::google_count_tokens::{decode::GoogleCountResponse, error::GoogleCountError};
use crate::model_urls::{ApiVersion, UrlMode};
use crate::token_refresh::CredentialRefresh;

pub struct VertexCountInput<'request> {
    pub model: &'request str,
    pub contents: &'request Json,
    // the Vertex override deliberately ignores the api_base, api_key and headers its caller supplies
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub headers: &'request BTreeMap<String, String>,
    pub params: VertexParams<'request>,
    pub settings: &'request VertexSettings,
    // LiteLLM accepts a timeout here and never forwards it; the deadline belongs to the pooled client
    pub timeout: Option<Duration>,
}

// LiteLLM's VertexAITokenCounter.validate_environment: B1301 settings and B1303 access tokens,
// then B1304's count_tokens URL, with only Bearer auth on the request
pub async fn count_tokens<T: CountTransport>(
    transport: &T,
    refresh: &CredentialRefresh,
    catalog: &ModelCatalog,
    input: &VertexCountInput<'_>,
    deadline: Duration,
) -> Result<GoogleCountResponse, GoogleCountError> {
    let credentials = input.params.safe_get_vertex_ai_credentials(input.settings);
    let configured_project = input.params.safe_get_vertex_ai_project(input.settings);
    let location = input.params.safe_get_vertex_ai_location(input.settings);
    let access = refresh
        .get_access_token_async(credentials, configured_project)
        .await
        .map_err(|source| GoogleCountError::Credentials { source })?;
    let target = get_token_and_url(
        catalog,
        &TokenUrlInput {
            provider: TargetProvider::Vertex,
            model: input.model,
            auth_token: Some(&access.token),
            gemini_api_key: None,
            project: Some(access.project.as_str()),
            location,
            stream: false,
            api_base: None,
            api_version: ApiVersion::V1,
            mode: UrlMode::CountTokens,
            use_psc_endpoint_format: false,
        },
    )
    .map_err(|source| GoogleCountError::Target { source })?;
    let headers = BTreeMap::from([(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {}", access.token.expose_secret())),
    )]);
    let request = CountRequest {
        url: target.url,
        headers: header_map(&headers)?,
        body: count_body(input.contents)?,
    };
    count(transport, request, deadline).await
}
