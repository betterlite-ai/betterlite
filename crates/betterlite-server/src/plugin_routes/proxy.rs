use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::{Extension, Path, Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::Response;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use serde::Deserialize;
use thiserror::Error;

use super::config::{PluginConfig, PluginRegistry};
use super::error::PluginError;
use super::headers::{KeyHeaderName, request_headers, response_headers};
use super::routes::{Identity, PROXY_ADMIN_ROLE, caller_identity};
use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

// the same 16 MiB the outbound client buffers, applied to the body we take in and forward
const REQUEST_BODY_LIMIT: usize = TransportLimits::DEFAULT.body_bytes;
const FORBIDDEN_TEXT: &str = "Plugin proxy access requires proxy_admin role.";
const TOO_LARGE_TEXT: &str = "Request body too large";

#[derive(Clone)]
pub struct PluginState {
    pub registry: Arc<PluginRegistry>,
    pub upstream: Arc<dyn PluginUpstream>,
    pub key_header: Option<Arc<dyn KeyHeaderName>>,
}

impl PluginState {
    #[must_use]
    pub fn production(
        registry: Arc<PluginRegistry>,
        environment: &BTreeMap<String, String>,
        key_header: Option<Arc<dyn KeyHeaderName>>,
    ) -> Self {
        Self {
            registry,
            upstream: Arc::new(ProxiedPluginUpstream::connect(environment)),
            key_header,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PluginRequest {
    pub method: Method,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct PluginReply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum PluginSendError {
    #[error("the plugin outbound client could not be built: {message}")]
    Unavailable { message: String },
    #[error("the plugin request could not be built: {message}")]
    Build { message: String },
    #[error("the plugin request failed: {source}")]
    Transport {
        #[source]
        source: HttpError,
    },
}

#[async_trait]
pub trait PluginUpstream: Send + Sync {
    async fn send(&self, request: PluginRequest) -> Result<PluginReply, PluginSendError>;
}

#[derive(Debug, Error)]
enum PluginClientError {
    #[error("the proxy environment is invalid: {source}")]
    Policy {
        #[source]
        source: EnvError,
    },
    #[error("the outbound client could not be built: {source}")]
    Client {
        #[source]
        source: HttpError,
    },
}

pub struct ProxiedPluginUpstream {
    client: Result<ProxiedClient, PluginClientError>,
}

impl ProxiedPluginUpstream {
    // A bad proxy environment is kept and reported when the next request is sent, like a failed client build in LiteLLM.
    #[must_use]
    pub fn connect(environment: &BTreeMap<String, String>) -> Self {
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), environment)
            .map(Arc::new)
            .map_err(PluginClientError::Policy);
        let client = policy.and_then(|resolved| {
            ProxiedClient::outbound(&resolved).map_err(PluginClientError::Client)
        });
        Self { client }
    }
}

#[async_trait]
impl PluginUpstream for ProxiedPluginUpstream {
    async fn send(&self, request: PluginRequest) -> Result<PluginReply, PluginSendError> {
        let client = self
            .client
            .as_ref()
            .map_err(|error| PluginSendError::Unavailable {
                message: error.to_string(),
            })?;
        let built = client
            .client
            .request(request.method, request.url)
            .headers(request.headers)
            .body(request.body)
            .build()
            .map_err(|error| PluginSendError::Build {
                message: error.to_string(),
            })?;
        // the client is built with redirect(Policy::none()), so a plugin's 3xx is answered and never followed
        let response = client
            .execute_buffered(built, client.limits.body_bytes)
            .await
            .map_err(|source| PluginSendError::Transport { source })?;
        Ok(PluginReply {
            status: StatusCode::from_u16(response.status).unwrap_or(StatusCode::BAD_GATEWAY),
            headers: response.headers,
            body: response.body,
        })
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProxyPath {
    plugin_name: String,
    #[serde(default)]
    path: String,
}

pub(crate) async fn plugin_proxy(
    State(state): State<PluginState>,
    caller: Option<Extension<AuthContext>>,
    Path(path): Path<ProxyPath>,
    request: Request,
) -> Response {
    let Some(authenticated) = caller else {
        return unauthenticated();
    };
    let identity = match caller_identity(&authenticated.principal) {
        Ok(identity) => identity,
        Err(error) => {
            return plain_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("The caller identity could not be read: {error}"),
            );
        }
    };
    // the role is checked before the registry lookup, so a non-admin asking for an unknown plugin still gets 403
    if identity.user_role.as_deref() != Some(PROXY_ADMIN_ROLE) {
        return plain_response(StatusCode::FORBIDDEN, FORBIDDEN_TEXT.to_owned());
    }
    forward_to_plugin(&state, &identity, &path, request).await
}

async fn forward_to_plugin(
    state: &PluginState,
    identity: &Identity,
    path: &ProxyPath,
    request: Request,
) -> Response {
    let request_method = request.method().clone();
    let query = request.uri().query().map(str::to_owned);
    let Some(plugin) = registered_plugin(state, &path.plugin_name) else {
        return plain_response(
            StatusCode::NOT_FOUND,
            format!("Plugin '{}' not registered", path.plugin_name),
        );
    };
    let unreachable = format!(
        "Cannot connect to plugin '{}' at {}",
        plugin.name, plugin.url
    );
    let forwarded = match plugin_headers(state, &request, &plugin, identity) {
        Ok(headers) => headers,
        Err(_error) => return plain_response(StatusCode::BAD_GATEWAY, unreachable),
    };
    let body = match to_bytes(request.into_body(), REQUEST_BODY_LIMIT).await {
        Ok(bytes) => bytes.to_vec(),
        Err(_error) => {
            return plain_response(StatusCode::PAYLOAD_TOO_LARGE, TOO_LARGE_TEXT.to_owned());
        }
    };
    let plugin_request = PluginRequest {
        method: request_method,
        url: target_url(plugin.url.as_str(), path.path.as_str(), query.as_deref()),
        headers: forwarded,
        body,
    };
    send_to_plugin(state, plugin_request, unreachable).await
}

fn registered_plugin(state: &PluginState, plugin_name: &str) -> Option<PluginConfig> {
    let plugins = state
        .registry
        .plugins
        .read()
        .unwrap_or_else(PoisonError::into_inner);
    plugins.get(plugin_name).cloned()
}

fn plugin_headers(
    state: &PluginState,
    request: &Request,
    plugin: &PluginConfig,
    identity: &Identity,
) -> Result<HeaderMap, PluginError> {
    let custom_key_header = state
        .key_header
        .as_ref()
        .and_then(|source| source.litellm_key_header_name());
    request_headers(
        request.headers(),
        custom_key_header.as_deref(),
        plugin.plugin_key.as_ref(),
        identity.user_id.as_deref(),
        identity.user_role.as_deref(),
    )
}

async fn send_to_plugin(
    state: &PluginState,
    plugin_request: PluginRequest,
    unreachable: String,
) -> Response {
    match state.upstream.send(plugin_request).await {
        Ok(reply) => {
            let mut response = Response::new(Body::from(reply.body));
            *response.status_mut() = reply.status;
            *response.headers_mut() = response_headers(&reply.headers);
            response
        }
        Err(_error) => plain_response(StatusCode::BAD_GATEWAY, unreachable),
    }
}

fn target_url(base: &str, path: &str, query: Option<&str>) -> String {
    let trimmed = base.trim_end_matches('/');
    let raw_query = query.unwrap_or("");
    let joined = format!("{trimmed}/{path}");
    if raw_query.is_empty() {
        joined
    } else {
        format!("{joined}?{raw_query}")
    }
}

fn plain_response(status: StatusCode, body: String) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response
}

#[cfg(test)]
mod tests;
