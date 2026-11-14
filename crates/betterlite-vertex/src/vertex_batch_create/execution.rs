use std::future::Future;
use std::str::from_utf8;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::api::batch_records::CreateBatchRequest;
use betterlite_openai::batches::records::BatchObject;
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::ExposeSecret as _;
use url::Url;
use uuid::Uuid;

use crate::cache_lookup::transport::CacheTransport;
use crate::endpoint_target::region::DEFAULT_LOCATION;
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_batch_records::create::to_batch_job;
use crate::vertex_batch_records::response::to_batch_object;
use crate::vertex_batch_records::types::{
    VertexAIBatchPredictionJob, VertexBatchPredictionResponse,
};

use super::error::VertexBatchCreateError;
use super::fetch::ProtectedGet;
use super::payload::{VertexEndpointResponse, decode_vertex_batch_payload};
use super::target::{collection_url, endpoint_resolution_url};

mod tests;

// LiteLLM's pooled httpx client waits 600 seconds, and the create call's own timeout never reaches it
pub const DEFAULT_CLIENT_DEADLINE: Duration = Duration::from_secs(600);

// the first 1000 characters of a failed create body are logged
const LOGGED_BODY_CHARS: usize = 1000;

pub struct BatchCreateRequest<'request> {
    pub create: &'request CreateBatchRequest,
    pub api_base: Option<&'request str>,
    pub project: Option<&'request str>,
    pub location: Option<&'request str>,
    pub custom_endpoint: bool,
    pub display_id: Uuid,
    pub deadline: Duration,
    // accepted for LiteLLM's signature and never forwarded to the provider call
    pub timeout: Option<f64>,
    pub max_retries: Option<u32>,
}

// everything that needs credentials or the request, resolved before any await on the network
#[derive(Debug)]
pub struct PreparedBatchCreate {
    job: VertexAIBatchPredictionJob,
    api_base: Option<String>,
    project: String,
    location: String,
    headers: HeaderMap,
    deadline: Duration,
}

// LiteLLM resolves the token synchronously even for an async create, so the lookup finishes here and execution only awaits I/O
pub async fn prepare_batch_create<Lookup>(
    request: &BatchCreateRequest<'_>,
    lookup: Lookup,
) -> Result<PreparedBatchCreate, VertexBatchCreateError>
where
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    if request.custom_endpoint {
        return Err(VertexBatchCreateError::CustomEndpoint);
    }
    let access = lookup
        .await
        .map_err(|source| VertexBatchCreateError::Credentials { source })?;
    let headers = bearer_headers(&access)?;
    // LiteLLM's `vertex_project or project_id` and `vertex_location or "us-central1"` treat an empty string as unset
    let project = request
        .project
        .filter(|project| !project.is_empty())
        .unwrap_or(&access.project)
        .to_owned();
    let location = request
        .location
        .filter(|location| !location.is_empty())
        .unwrap_or(DEFAULT_LOCATION)
        .to_owned();
    let job = to_batch_job(
        request.create,
        Some(&project),
        Some(&location),
        request.display_id,
    )
    .map_err(|source| VertexBatchCreateError::Records { source })?;
    Ok(PreparedBatchCreate {
        job,
        api_base: request.api_base.map(str::to_owned),
        project,
        location,
        headers,
        deadline: request.deadline,
    })
}

fn bearer_headers(access: &AccessToken) -> Result<HeaderMap, VertexBatchCreateError> {
    let mut authorization =
        HeaderValue::from_str(&format!("Bearer {}", access.token.expose_secret()))
            .map_err(|source| VertexBatchCreateError::Header { source })?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    headers.insert(AUTHORIZATION, authorization);
    Ok(headers)
}

pub async fn execute_batch_create<Transport, Fetch>(
    transport: &Transport,
    fetcher: &Fetch,
    mut prepared: PreparedBatchCreate,
    local: &LocalTimeContext,
) -> Result<BatchObject, VertexBatchCreateError>
where
    Transport: CacheTransport,
    Fetch: ProtectedGet,
{
    prepared.job.model = resolve_model(fetcher, &prepared).await?;
    let created = post_batch_job(transport, prepared).await?;
    to_batch_object(&created, local).map_err(|source| VertexBatchCreateError::Records { source })
}

async fn post_batch_job<Transport: CacheTransport>(
    transport: &Transport,
    prepared: PreparedBatchCreate,
) -> Result<VertexBatchPredictionResponse, VertexBatchCreateError> {
    let collection = collection_url(
        prepared.api_base.as_deref(),
        &prepared.project,
        &prepared.location,
    )?;
    let body = serde_json::to_vec(&prepared.job)
        .map_err(|source| VertexBatchCreateError::Encode { source })?;
    let reply = transport
        .send(OutboundRequest {
            method: Method::POST,
            url: Url::parse(&collection)
                .map_err(|source| VertexBatchCreateError::Url { source })?,
            query: Vec::new(),
            headers: prepared.headers,
            body: RequestBody::Json(body),
            deadline: prepared.deadline,
        })
        .await
        .map_err(VertexBatchCreateError::from_transport)?;
    // httpx's raise_for_status ends the call on a 4xx or 5xx and returns every other status
    if reply.status >= 400 {
        let message = body_text(&reply.body);
        tracing::error!(
            "Vertex AI batch create failed: status={}, body={}",
            reply.status,
            message.chars().take(LOGGED_BODY_CHARS).collect::<String>()
        );
        return Err(VertexBatchCreateError::Http {
            status: reply.status,
            message,
        });
    }
    decode_vertex_batch_payload(&reply.body)
}

// httpx decodes response.text leniently, so a body that is not UTF-8 keeps the text before its first invalid byte
fn body_text(body: &[u8]) -> String {
    match from_utf8(body) {
        Ok(text) => text.to_owned(),
        Err(invalid) => from_utf8(body.get(..invalid.valid_up_to()).unwrap_or_default())
            .unwrap_or_default()
            .to_owned(),
    }
}

// the v1 batch API only takes Model resources, so an endpoint resource is swapped for the first deployed model it lists
async fn resolve_model<Fetch: ProtectedGet>(
    fetcher: &Fetch,
    prepared: &PreparedBatchCreate,
) -> Result<String, VertexBatchCreateError> {
    let model = &prepared.job.model;
    if !model.contains("/endpoints/") {
        return Ok(model.clone());
    }
    let endpoint =
        endpoint_resolution_url(prepared.api_base.as_deref(), model, &prepared.location)?;
    let reply = fetcher
        .get(&endpoint, &prepared.headers, prepared.deadline)
        .await?;
    if reply.status != 200 {
        return Err(VertexBatchCreateError::EndpointStatus {
            status: reply.status,
            model: model.clone(),
            message: body_text(&reply.body),
        });
    }
    let described: VertexEndpointResponse = decode_vertex_batch_payload(&reply.body)?;
    // only the first deployed model counts, and an empty or missing one is never replaced by a later entry
    described
        .deployed_models
        .unwrap_or_default()
        .into_iter()
        .next()
        .and_then(|deployed| deployed.model)
        .filter(|deployed| !deployed.is_empty())
        .ok_or_else(|| VertexBatchCreateError::NoDeployedModel {
            model: model.clone(),
        })
}
