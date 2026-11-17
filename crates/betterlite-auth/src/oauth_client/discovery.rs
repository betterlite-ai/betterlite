use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::HeaderMap;

use crate::oauth_client::document::{OAuthDiscoveryDocument, decode_oauth_provider_document};
use crate::oauth_client::error::{
    OAuthClientError, OAuthEndpoint, OAuthProviderFailure, endpoint_url, require_secure,
};
use crate::oauth_client::{OAUTH_REQUEST_TIMEOUT, OAuthClient};

#[derive(Debug, Clone)]
pub enum DiscoverySource {
    WellKnown(String),
    Inline(OAuthDiscoveryDocument),
}

pub async fn fetch_discovery(
    client: &OAuthClient,
    transport: &ProxiedClient,
    source: &DiscoverySource,
) -> Result<OAuthDiscoveryDocument, OAuthClientError> {
    let text = match source {
        DiscoverySource::Inline(document) => return Ok(document.clone()),
        DiscoverySource::WellKnown(text) => text,
    };
    let url = endpoint_url(text, OAuthEndpoint::Discovery)?;
    require_secure(client, &url, OAuthEndpoint::Discovery)?;
    let response = transport
        .send_buffered(OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
            deadline: OAUTH_REQUEST_TIMEOUT,
        })
        .await
        .map_err(|source| OAuthClientError::Discovery(OAuthProviderFailure::Transport(source)))?;
    if !(200..=299).contains(&response.status) {
        return Err(OAuthClientError::Discovery(OAuthProviderFailure::Status {
            status: response.status,
        }));
    }
    decode_oauth_provider_document(&response.body)
        .map_err(|source| OAuthClientError::Discovery(OAuthProviderFailure::Document(source)))
}
