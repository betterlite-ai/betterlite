use std::future::Future;
use std::str::from_utf8;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::HeaderMap;

use crate::cache_lookup::transport::CacheTransport;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_batch_create::execution::DEFAULT_CLIENT_DEADLINE;

use super::error::VertexBatchLifecycleError;
use super::wire::{bearer_headers, get_request, project_and_location};

mod tests;

// the kwargs every lifecycle signature takes; only the cancel POST forwards its timeout to the provider call
pub struct BatchLifecycleRequest<'request> {
    pub api_base: Option<&'request str>,
    pub project: Option<&'request str>,
    pub location: Option<&'request str>,
    pub timeout: Option<Duration>,
    pub max_retries: Option<u32>,
}

pub(super) struct ResolvedRequest {
    pub(super) access: AccessToken,
    pub(super) project: String,
    pub(super) location: String,
    pub(super) headers: HeaderMap,
}

pub(super) async fn resolve<Lookup>(
    request: &BatchLifecycleRequest<'_>,
    lookup: Lookup,
) -> Result<ResolvedRequest, VertexBatchLifecycleError>
where
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let access = lookup
        .await
        .map_err(|source| VertexBatchLifecycleError::Credentials { source })?;
    let (project, location) = project_and_location(request, &access);
    let headers = bearer_headers(&access)?;
    Ok(ResolvedRequest {
        access,
        project,
        location,
        headers,
    })
}

pub(super) async fn send_get<Transport>(
    transport: &Transport,
    target: &str,
    headers: HeaderMap,
    query: Vec<(String, String)>,
    transport_error: fn(TransportError) -> VertexBatchLifecycleError,
) -> Result<UpstreamResponse, VertexBatchLifecycleError>
where
    Transport: CacheTransport,
{
    transport
        .send(get_request(
            target,
            headers,
            query,
            DEFAULT_CLIENT_DEADLINE,
        )?)
        .await
        .map_err(transport_error)
}

// httpx's response.text replaces invalid bytes, so a message keeps the text before the first one
pub(super) fn body_text(body: &[u8]) -> String {
    match from_utf8(body) {
        Ok(text) => text.to_owned(),
        Err(invalid) => from_utf8(body.get(..invalid.valid_up_to()).unwrap_or_default())
            .unwrap_or_default()
            .to_owned(),
    }
}
