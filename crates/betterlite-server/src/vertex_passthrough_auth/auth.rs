use std::sync::Arc;

use betterlite_auth::passthrough::caller_headers::caller_filter::FilterSettings;
use betterlite_auth::passthrough::caller_headers::error::CallerHeaderError;
use betterlite_auth::passthrough::caller_headers::upstream::vertex_credentialless_headers;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_logging::python_json::dumps_object;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_vertex::credential_sources::error::CredentialSourceError;
use betterlite_vertex::credential_sources::sdk::CredentialClock;
use betterlite_vertex::endpoint_target::error::EndpointTargetError;
use betterlite_vertex::endpoint_target::target::{
    AuthHeader, EndpointTarget, TargetProvider, TokenUrlInput, get_token_and_url,
};
use betterlite_vertex::model_policy::capabilities::VertexCapabilityProvider;
use betterlite_vertex::model_urls::{ApiVersion, UrlMode};
use betterlite_vertex::token_cache::CredentialCache;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::executor::CredentialExecutor;
use betterlite_vertex::token_refresh::{CredentialRefresh, ensure_access_token};
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::task::TaskTracker;
use tracing::debug;

use super::headers::allowed_vertex_headers;
use super::override_params::RouterCredentials;
use crate::pass_through_credentials::VertexPassThroughCredentials;

pub struct VertexAuthInput<'request> {
    pub request: &'request HeaderMap,
    pub vertex_credentials: Option<VertexPassThroughCredentials>,
    pub router_credentials: Option<&'request RouterCredentials>,
    pub project: Option<String>,
    pub location: Option<String>,
    pub user_api_key: &'request UserAPIKeyAuth,
}

// LiteLLM builds a VertexBase per call, so the cache is fresh per call; the tracker still belongs to the server
pub struct VertexAuthDeps<'deps> {
    pub clock: Arc<dyn CredentialClock>,
    pub executor: Arc<dyn CredentialExecutor>,
    pub tasks: TaskTracker,
    pub filters: FilterSettings<'deps>,
}

#[derive(Debug)]
pub struct VertexAuthPrep {
    pub headers: IndexMap<String, SecretString>,
    pub headers_passed_through: bool,
    pub project: Option<String>,
    pub location: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum VertexAuthError {
    #[error(transparent)]
    CallerHeaders(#[from] CallerHeaderError),
    #[error(transparent)]
    Credentials(#[from] CredentialSourceError),
    #[error("vertex_credentials must be a string or an object")]
    CredentialsType,
    #[error(transparent)]
    Token(#[from] CredentialCacheError),
    #[error(transparent)]
    Target(#[from] EndpointTargetError),
}

enum AuthSource<'source> {
    CallerHeaders,
    DefaultCredentials,
    Configured(&'source VertexPassThroughCredentials),
}

// The first branch covers no credentials and a record without a project; only a configured project reaches branch two
const fn auth_source<'source>(
    vertex_credentials: Option<&'source VertexPassThroughCredentials>,
    router_credentials: Option<&'source RouterCredentials>,
) -> AuthSource<'source> {
    if router_credentials.is_some() {
        return AuthSource::DefaultCredentials;
    }
    match vertex_credentials {
        Some(record) if record.vertex_project.is_some() => AuthSource::Configured(record),
        Some(_) | None => AuthSource::CallerHeaders,
    }
}

struct TokenRequest {
    credentials: Option<ConfiguredCredentials>,
    project: Option<String>,
    location: Option<String>,
}

pub async fn prepare_vertex_auth_headers(
    VertexAuthInput {
        request,
        vertex_credentials,
        router_credentials,
        project,
        location,
        user_api_key,
    }: VertexAuthInput<'_>,
    deps: &VertexAuthDeps<'_>,
) -> Result<VertexAuthPrep, VertexAuthError> {
    match auth_source(vertex_credentials.as_ref(), router_credentials) {
        AuthSource::CallerHeaders => {
            let headers = vertex_credentialless_headers(request, user_api_key, &deps.filters)?;
            let names: Vec<&str> = headers.keys().map(String::as_str).collect();
            debug!(
                header_names = ?names,
                "default_vertex_config not set, forwarding caller-provided headers"
            );
            Ok(VertexAuthPrep {
                headers,
                headers_passed_through: true,
                project,
                location,
            })
        }
        AuthSource::DefaultCredentials => {
            let token_request = TokenRequest {
                credentials: None,
                project,
                location,
            };
            token_headers(token_request, request, deps).await
        }
        AuthSource::Configured(record) => {
            let credentials = configured_credentials(record.vertex_credentials.as_ref())?;
            let token_request = TokenRequest {
                credentials,
                project: record.vertex_project.clone(),
                location: record.vertex_location.clone().or(location),
            };
            token_headers(token_request, request, deps).await
        }
    }
}

fn configured_credentials(
    value: Option<&Json>,
) -> Result<Option<ConfiguredCredentials>, VertexAuthError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(decoded(text)?)),
        Some(Json::Object(object)) => Ok(Some(decoded(&dumps_object(object))?)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            Err(VertexAuthError::CredentialsType)
        }
    }
}

fn decoded(text: &str) -> Result<ConfiguredCredentials, VertexAuthError> {
    let configured = ConfiguredCredentials::from_text(text.to_owned())
        .map_err(|source| CredentialSourceError::Credentials { source })?;
    Ok(configured)
}

async fn token_headers(
    request: TokenRequest,
    headers: &HeaderMap,
    deps: &VertexAuthDeps<'_>,
) -> Result<VertexAuthPrep, VertexAuthError> {
    let cache = Arc::new(CredentialCache::new(
        Arc::clone(&deps.clock),
        Arc::clone(&deps.executor),
    ));
    let refresh = CredentialRefresh::new(cache, deps.tasks.clone());
    // custom_llm_provider is the literal vertex_ai_beta this route passes
    let access = ensure_access_token(
        VertexCapabilityProvider::VertexAiBeta,
        refresh.get_access_token_async(request.credentials.as_ref(), request.project.as_deref()),
    )
    .await?;
    let project = Some(access.project.clone());
    let target = vertex_token_target(
        &access.token,
        project.as_deref(),
        request.location.as_deref(),
    )?;
    // The first element is the token handed in; with no custom api_base a Headers auth cannot occur
    let token = match target.auth {
        Some(AuthHeader::Token(token)) => token,
        Some(AuthHeader::Headers(_)) | None => access.token,
    };
    let bearer = token.expose_secret();
    let mut prepared = allowed_vertex_headers(headers);
    prepared.insert(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {bearer}")),
    );
    Ok(VertexAuthPrep {
        headers: prepared,
        headers_passed_through: false,
        project,
        location: request.location,
    })
}

// LiteLLM's _get_token_and_url with model "", stream false, api_base "" and version v1; only the auth token is kept
pub(super) fn vertex_token_target(
    token: &SecretString,
    project: Option<&str>,
    location: Option<&str>,
) -> Result<EndpointTarget, EndpointTargetError> {
    get_token_and_url(
        &ModelCatalog::default(),
        &TokenUrlInput {
            provider: TargetProvider::Vertex,
            model: "",
            auth_token: Some(token),
            gemini_api_key: None,
            project,
            location,
            stream: false,
            api_base: Some(""),
            api_version: ApiVersion::V1,
            mode: UrlMode::Chat,
            use_psc_endpoint_format: false,
        },
    )
}
