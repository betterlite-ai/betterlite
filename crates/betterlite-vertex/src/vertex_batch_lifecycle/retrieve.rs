use std::future::Future;

use betterlite_http::url_path::encode_segment;
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_openai::batches::records::BatchObject;
use http::header::HeaderMap;

use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_batch_create::execution::{DEFAULT_CLIENT_DEADLINE, body_text};
use crate::vertex_batch_create::fetch::ProtectedGet;
use crate::vertex_batch_create::payload::decode_vertex_batch_payload;
use crate::vertex_batch_create::target::collection_url;
use crate::vertex_batch_records::response::to_batch_object;
use crate::vertex_batch_records::types::VertexBatchPredictionResponse;

use super::error::VertexBatchLifecycleError;
use super::execution::{BatchLifecycleRequest, resolve};
use super::target::{endpoint_tail, proxy_url};
use super::wire::pre_call_args;

pub struct PreparedBatchRetrieve {
    target: String,
    headers: HeaderMap,
    pub logging: AdditionalArgs,
}

pub fn retrieve_url(
    api_base: Option<&str>,
    project: &str,
    location: &str,
    batch_id: &str,
) -> Result<String, VertexBatchLifecycleError> {
    let encoded = encode_segment(batch_id, "batch_id")?;
    let default = format!("{}/{}", collection_url(None, project, location)?, encoded);
    let endpoint = endpoint_tail(&default);
    proxy_url(api_base, &default, endpoint, Some(project), Some(location))
}

pub async fn prepare_batch_retrieve<Lookup>(
    request: &BatchLifecycleRequest<'_>,
    batch_id: &str,
    lookup: Lookup,
) -> Result<PreparedBatchRetrieve, VertexBatchLifecycleError>
where
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let resolved = resolve(request, lookup).await?;
    let target = retrieve_url(
        request.api_base,
        &resolved.project,
        &resolved.location,
        batch_id,
    )?;
    let logging = pre_call_args(&target, &resolved.access);
    Ok(PreparedBatchRetrieve {
        target,
        headers: resolved.headers,
        logging,
    })
}

pub async fn execute_batch_retrieve<Fetch>(
    fetcher: &Fetch,
    prepared: PreparedBatchRetrieve,
    local: &LocalTimeContext,
) -> Result<BatchObject, VertexBatchLifecycleError>
where
    Fetch: ProtectedGet,
{
    // LiteLLM's safe_get has no handler, so a timeout there reaches the proxy without a status of its own
    let reply = fetcher
        .get(&prepared.target, &prepared.headers, DEFAULT_CLIENT_DEADLINE)
        .await
        .map_err(|source| VertexBatchLifecycleError::ProtectedGet { source })?;
    if reply.status != 200 {
        return Err(VertexBatchLifecycleError::RetrieveStatus {
            status: reply.status,
            message: body_text(&reply.body),
        });
    }
    let response: VertexBatchPredictionResponse = decode_vertex_batch_payload(&reply.body)?;
    to_batch_object(&response, local)
        .map_err(|source| VertexBatchLifecycleError::Records { source })
}
