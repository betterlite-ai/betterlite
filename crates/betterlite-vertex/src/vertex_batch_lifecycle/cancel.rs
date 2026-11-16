use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::url_path::encode_segment;
use betterlite_openai::batches::records::BatchObject;
use http::Method;
use http::header::HeaderMap;
use url::Url;

use crate::cache_lookup::transport::CacheTransport;
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_batch_create::execution::DEFAULT_CLIENT_DEADLINE;
use crate::vertex_batch_create::payload::decode_vertex_batch_payload;
use crate::vertex_batch_create::target::collection_url;
use crate::vertex_batch_records::response::to_batch_object;
use crate::vertex_batch_records::types::VertexBatchPredictionResponse;

use super::error::VertexBatchLifecycleError;
use super::execution::{BatchLifecycleRequest, body_text, resolve, send_get};
use super::target::proxy_url;

// LiteLLM posts json.dumps({}) to the cancel endpoint
const CANCEL_BODY: &[u8] = b"{}";

// a failed cancel or its follow-up logs the first 1000 characters of the body
const LOGGED_BODY_CHARS: usize = 1000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelTarget {
    pub cancel: String,
    pub retrieve: String,
}

pub struct PreparedBatchCancel {
    cancel: String,
    retrieve: String,
    headers: HeaderMap,
    post_deadline: Duration,
}

pub fn cancel_target(
    api_base: Option<&str>,
    project: &str,
    location: &str,
    batch_id: &str,
) -> Result<CancelTarget, VertexBatchLifecycleError> {
    let encoded = encode_segment(batch_id, "batch_id")?;
    let default = format!(
        "{}/{}:cancel",
        collection_url(None, project, location)?,
        encoded
    );
    let cancel = proxy_url(api_base, &default, "cancel", Some(project), Some(location))?;
    let retrieve = retrieval_url(&cancel);
    Ok(CancelTarget { cancel, retrieve })
}

// LiteLLM removes one terminal ":cancel" as text, and only the fallback path trims slashes off the remainder
pub(super) fn retrieval_url(cancel_url: &str) -> String {
    match cancel_url.strip_suffix(":cancel") {
        Some(retrieve) => retrieve.to_owned(),
        None => cancel_url
            .rsplit_once(":cancel")
            .map_or(cancel_url, |(head, _tail)| head)
            .trim_end_matches('/')
            .to_owned(),
    }
}

pub async fn prepare_batch_cancel<Lookup>(
    request: &BatchLifecycleRequest<'_>,
    batch_id: &str,
    lookup: Lookup,
) -> Result<PreparedBatchCancel, VertexBatchLifecycleError>
where
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let resolved = resolve(request, lookup).await?;
    let target = cancel_target(
        request.api_base,
        &resolved.project,
        &resolved.location,
        batch_id,
    )?;
    let post_deadline = request.timeout.unwrap_or(DEFAULT_CLIENT_DEADLINE);
    Ok(PreparedBatchCancel {
        cancel: target.cancel,
        retrieve: target.retrieve,
        headers: resolved.headers,
        post_deadline,
    })
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
    let reply = send_get(
        transport,
        &prepared.retrieve,
        prepared.headers,
        Vec::new(),
        |source| VertexBatchLifecycleError::RetrieveTransport { source },
    )
    .await?;
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
