use std::future::Future;

use http::header::HeaderMap;

use crate::cache_lookup::transport::CacheTransport;
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_batch_create::target::collection_url;
use crate::vertex_batch_records::response::{BatchList, to_batch_list};

use super::error::VertexBatchLifecycleError;
use super::execution::{BatchLifecycleRequest, resolve, send_get};
use super::page::decode_vertex_batch_page;
use super::target::{endpoint_tail, proxy_url};
use super::wire::list_query;

pub struct PreparedBatchList {
    target: String,
    headers: HeaderMap,
    query: Vec<(String, String)>,
}

// LiteLLM's list call omits the project, location and version arguments, and its non-PSC path reads none of them
pub fn list_url(
    api_base: Option<&str>,
    project: &str,
    location: &str,
) -> Result<String, VertexBatchLifecycleError> {
    let default = collection_url(None, project, location)?;
    let endpoint = endpoint_tail(&default);
    proxy_url(api_base, &default, endpoint, None, None)
}

pub async fn prepare_batch_list<Lookup>(
    request: &BatchLifecycleRequest<'_>,
    after: Option<&str>,
    limit: Option<i64>,
    lookup: Lookup,
) -> Result<PreparedBatchList, VertexBatchLifecycleError>
where
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let resolved = resolve(request, lookup).await?;
    let target = list_url(request.api_base, &resolved.project, &resolved.location)?;
    let query = list_query(after, limit);
    Ok(PreparedBatchList {
        target,
        headers: resolved.headers,
        query,
    })
}

pub async fn execute_batch_list<Transport>(
    transport: &Transport,
    prepared: PreparedBatchList,
    local: &LocalTimeContext,
) -> Result<BatchList, VertexBatchLifecycleError>
where
    Transport: CacheTransport,
{
    let reply = send_get(
        transport,
        &prepared.target,
        prepared.headers,
        prepared.query,
        VertexBatchLifecycleError::ListTransport,
    )
    .await?;
    let page = decode_vertex_batch_page(&reply)?;
    to_batch_list(&page, local).map_err(|source| VertexBatchLifecycleError::Records { source })
}
