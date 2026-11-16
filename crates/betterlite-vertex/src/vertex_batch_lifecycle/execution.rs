use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_openai::batches::records::BatchObject;
use http::Method;
use http::header::HeaderMap;
use url::Url;

use crate::cache_lookup::transport::CacheTransport;
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_batch_create::execution::DEFAULT_CLIENT_DEADLINE;
use crate::vertex_batch_create::fetch::ProtectedGet;
use crate::vertex_batch_create::payload::decode_vertex_batch_payload;
use crate::vertex_batch_records::response::{BatchList, to_batch_list, to_batch_object};
use crate::vertex_batch_records::types::VertexBatchPredictionResponse;

use super::error::VertexBatchLifecycleError;
use super::page::{body_text, decode_vertex_batch_page};
use super::target::{cancel_target, list_url, retrieve_url};
use super::wire::{bearer_headers, get_request, list_query, pre_call_args, project_and_location};

mod tests;

// LiteLLM posts json.dumps({}) to the cancel endpoint
const CANCEL_BODY: &[u8] = b"{}";

// a failed cancel logs the first 1000 characters of the provider body
const LOGGED_BODY_CHARS: usize = 1000;

// the kwargs every lifecycle signature takes; only the cancel POST forwards its timeout to the provider call
pub struct BatchLifecycleRequest<'request> {
    pub api_base: Option<&'request str>,
    pub project: Option<&'request str>,
    pub location: Option<&'request str>,
    pub timeout: Option<Duration>,
    pub max_retries: Option<u32>,
}

pub struct PreparedBatchRetrieve {
    target: String,
    headers: HeaderMap,
    pub logging: AdditionalArgs,
}

pub struct PreparedBatchList {
    target: String,
    headers: HeaderMap,
    query: Vec<(String, String)>,
}

pub struct PreparedBatchCancel {
    cancel: String,
    retrieve: String,
    headers: HeaderMap,
    post_deadline: Duration,
}

pub async fn prepare_batch_retrieve<Lookup>(
    request: &BatchLifecycleRequest<'_>,
    batch_id: &str,
    lookup: Lookup,
) -> Result<PreparedBatchRetrieve, VertexBatchLifecycleError>
where
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let (access, headers) = credentials(lookup).await?;
    let (project, location) = project_and_location(request, &access);
    let target = retrieve_url(request.api_base, &project, &location, batch_id)?;
    let logging = pre_call_args(&target, &access);
    Ok(PreparedBatchRetrieve {
        target,
        headers,
        logging,
    })
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
    let (access, headers) = credentials(lookup).await?;
    let (project, location) = project_and_location(request, &access);
    let target = list_url(request.api_base, &project, &location)?;
    let query = list_query(after, limit);
    Ok(PreparedBatchList {
        target,
        headers,
        query,
    })
}

pub async fn prepare_batch_cancel<Lookup>(
    request: &BatchLifecycleRequest<'_>,
    batch_id: &str,
    lookup: Lookup,
) -> Result<PreparedBatchCancel, VertexBatchLifecycleError>
where
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let (access, headers) = credentials(lookup).await?;
    let (project, location) = project_and_location(request, &access);
    let target = cancel_target(request.api_base, &project, &location, batch_id)?;
    let post_deadline = request.timeout.unwrap_or(DEFAULT_CLIENT_DEADLINE);
    Ok(PreparedBatchCancel {
        cancel: target.cancel,
        retrieve: target.retrieve,
        headers,
        post_deadline,
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

pub async fn execute_batch_list<Transport>(
    transport: &Transport,
    prepared: PreparedBatchList,
    local: &LocalTimeContext,
) -> Result<BatchList, VertexBatchLifecycleError>
where
    Transport: CacheTransport,
{
    let request = get_request(
        &prepared.target,
        prepared.headers,
        prepared.query,
        DEFAULT_CLIENT_DEADLINE,
    )?;
    let reply = transport
        .send(request)
        .await
        .map_err(|source| VertexBatchLifecycleError::ListTransport { source })?;
    let page = decode_vertex_batch_page(&reply)?;
    to_batch_list(&page, local).map_err(|source| VertexBatchLifecycleError::Records { source })
}

pub async fn execute_batch_cancel<Transport>(
    transport: &Transport,
    prepared: PreparedBatchCancel,
    local: &LocalTimeContext,
) -> Result<BatchObject, VertexBatchLifecycleError>
where
    Transport: CacheTransport,
{
    post_cancel(transport, &prepared).await?;
    let request = get_request(
        &prepared.retrieve,
        prepared.headers,
        Vec::new(),
        DEFAULT_CLIENT_DEADLINE,
    )?;
    let reply = transport
        .send(request)
        .await
        .map_err(|source| VertexBatchLifecycleError::RetrieveTransport { source })?;
    if reply.status != 200 {
        let message = body_text(&reply.body);
        tracing::error!(
            "Vertex AI batch retrieve-after-cancel failed: status={}, body={}",
            reply.status,
            message.chars().take(LOGGED_BODY_CHARS).collect::<String>()
        );
        return Err(VertexBatchLifecycleError::RetrieveAfterCancelStatus {
            status: reply.status,
            message,
        });
    }
    let response: VertexBatchPredictionResponse = decode_vertex_batch_payload(&reply.body)?;
    to_batch_object(&response, local)
        .map_err(|source| VertexBatchLifecycleError::Records { source })
}

async fn post_cancel<Transport>(
    transport: &Transport,
    prepared: &PreparedBatchCancel,
) -> Result<(), VertexBatchLifecycleError>
where
    Transport: CacheTransport,
{
    let url =
        Url::parse(&prepared.cancel).map_err(|source| VertexBatchLifecycleError::Url { source })?;
    let reply = transport
        .send(OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers: prepared.headers.clone(),
            body: RequestBody::Json(CANCEL_BODY.to_vec()),
            deadline: prepared.post_deadline,
        })
        .await
        .map_err(VertexBatchLifecycleError::cancel_transport)?;
    if (200..300).contains(&reply.status) {
        return Ok(());
    }
    let message = body_text(&reply.body);
    tracing::error!(
        "Vertex AI batch cancel failed: status={}, body={}",
        reply.status,
        message.chars().take(LOGGED_BODY_CHARS).collect::<String>()
    );
    Err(VertexBatchLifecycleError::CancelStatus {
        status: reply.status,
        message,
    })
}

async fn credentials<Lookup>(
    lookup: Lookup,
) -> Result<(AccessToken, HeaderMap), VertexBatchLifecycleError>
where
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let access = lookup
        .await
        .map_err(|source| VertexBatchLifecycleError::Credentials { source })?;
    let headers = bearer_headers(&access)?;
    Ok((access, headers))
}
