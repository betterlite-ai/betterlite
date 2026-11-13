use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::studio_identity::Settings as StudioSettings;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::catalog::ModelCatalog;
use secrecy::SecretString;

use crate::credential_sources::getters::VertexParams;
use crate::endpoint_target::headers::set_headers;
use crate::endpoint_target::target::{
    EndpointTarget, TargetProvider, TokenUrlInput, get_token_and_url,
};
use crate::model_urls::{ApiVersion, UrlMode};
use crate::studio_identity::{AuthValue, NativeParams, native_api_key, native_headers};
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::token_refresh::ensure_access_token;

use super::capabilities::NativeProvider;
use super::error::NativeContentError;

// LiteLLM's _get_common_auth_components reads through the mutable Vertex getters in this order
pub struct VertexComponents<'text> {
    pub credentials: Option<&'text ConfiguredCredentials>,
    pub project: Option<&'text str>,
    pub location: Option<&'text str>,
}

#[must_use]
pub fn vertex_components<'text>(
    params: VertexParams<'text>,
    settings: &'text VertexSettings,
) -> VertexComponents<'text> {
    VertexComponents {
        credentials: params.safe_get_vertex_ai_credentials(settings),
        project: params.safe_get_vertex_ai_project(settings),
        location: params.safe_get_vertex_ai_location(settings),
    }
}

pub struct NativeAuth<'text> {
    pub provider: NativeProvider,
    pub model: &'text str,
    pub api_base: Option<&'text str>,
    pub stream: bool,
    pub components: VertexComponents<'text>,
    pub studio: &'text StudioSettings,
    pub studio_keys: NativeParams,
}

#[derive(Debug)]
pub struct AuthTarget {
    pub url: String,
    pub headers: BTreeMap<String, SecretString>,
}

pub async fn auth_token_and_url<Lookup>(
    catalog: &ModelCatalog,
    auth: &NativeAuth<'_>,
    lookup: Lookup,
) -> Result<AuthTarget, NativeContentError>
where
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let access = ensure_access_token(auth.provider.capability_provider(), lookup)
        .await
        .map_err(|source| NativeContentError::Credentials { source })?;
    let key = match auth.provider {
        NativeProvider::Studio => Some(studio_key(auth)?),
        NativeProvider::Vertex => None,
    };
    let target = native_target(catalog, auth, key.as_ref(), &access)?;
    let headers = headers_for(auth, key, &access);
    Ok(AuthTarget {
        url: target.url,
        headers,
    })
}

fn native_target(
    catalog: &ModelCatalog,
    auth: &NativeAuth<'_>,
    key: Option<&SecretString>,
    access: &AccessToken,
) -> Result<EndpointTarget, NativeContentError> {
    // _get_token_and_url runs with should_use_v1beta1_features=True, so the URL keeps v1beta1
    get_token_and_url(
        catalog,
        &TokenUrlInput {
            provider: match auth.provider {
                NativeProvider::Studio => TargetProvider::Gemini,
                NativeProvider::Vertex => TargetProvider::Vertex,
            },
            model: auth.model,
            auth_token: Some(&access.token),
            gemini_api_key: key,
            project: Some(access.project.as_str()),
            location: auth.components.location,
            stream: auth.stream,
            api_base: auth.api_base,
            api_version: ApiVersion::V1Beta1,
            mode: UrlMode::Chat,
            use_psc_endpoint_format: false,
        },
    )
    .map_err(|source| NativeContentError::Url { source })
}

// _build_final_headers_and_url hands the key or token to the provider's own validate_environment
fn headers_for(
    auth: &NativeAuth<'_>,
    key: Option<SecretString>,
    access: &AccessToken,
) -> BTreeMap<String, SecretString> {
    match auth.provider {
        NativeProvider::Studio => native_headers(
            key.map(AuthValue::Key).as_ref(),
            &auth.studio_keys,
            auth.studio,
            &BTreeMap::new(),
        ),
        NativeProvider::Vertex => set_headers(Some(&access.token), &BTreeMap::new()),
    }
}

fn studio_key(auth: &NativeAuth<'_>) -> Result<SecretString, NativeContentError> {
    match native_api_key(None, &auth.studio_keys, auth.studio) {
        Some(AuthValue::Key(key)) => Ok(key),
        Some(AuthValue::Headers(_)) => Err(NativeContentError::StudioKeyNotText),
        None => Err(NativeContentError::MissingStudioKey),
    }
}
