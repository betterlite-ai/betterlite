use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::{Extra, Json};
use secrecy::SecretString;

use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::multimodal_embedding_records::request::transform_request;
use crate::multimodal_embedding_records::response::MultimodalEmbeddingAnswer;
use crate::multimodal_embedding_records::response::transform_response;
use crate::token_cache::CredentialCache;
use crate::token_refresh::ensure_access_token;

use super::error::VertexEmbeddingCallError;
use super::logging;
use super::post::{ClientTimeout, EmbeddingPost, HANDLER_DEFAULT_TIMEOUT, PostRequest, Timeout};
use super::target::{self, HeaderMode, TargetRequest};

// multimodal_embedding's signature defaults timeout to 300 seconds, and main.py passes none
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

// LiteLLM's VertexMultimodalEmbedding supported-model list, matched by name exactly as main.py does
pub const MULTIMODAL_EMBEDDING_MODELS: [&str; 2] =
    ["multimodalembedding", "multimodalembedding@001"];

pub struct MultimodalCall<'call> {
    pub provider: VertexCapabilityProvider,
    pub model: &'call str,
    pub input: &'call Json,
    pub optional_params: &'call Extra,
    pub headers: &'call BTreeMap<String, String>,
    pub api_base: Option<&'call str>,
    pub gemini_api_key: Option<&'call SecretString>,
    pub project: Option<&'call str>,
    pub location: Option<&'call str>,
    pub credentials: Option<&'call ConfiguredCredentials>,
    pub timeout: Option<Duration>,
    pub aembedding: bool,
}

pub struct MultimodalClients<'clients> {
    pub pooled: &'clients dyn EmbeddingPost,
    pub caller: Option<&'clients dyn EmbeddingPost>,
}

// The posted call both branches read: the forced headers and body, and the payload the reply's usage is calculated from
pub struct MultimodalRequest {
    pub url: String,
    pub headers: BTreeMap<String, SecretString>,
    pub body: Vec<u8>,
    pub payload: Json,
}

// A numeric timeout becomes httpx.Timeout(timeout, connect=timeout); None takes the timeout the branch configures for itself
#[must_use]
pub(super) const fn multimodal_timeout(
    timeout: Option<Duration>,
    fallback: ClientTimeout,
) -> ClientTimeout {
    match timeout {
        Some(total) => ClientTimeout::Explicit(Timeout {
            total,
            connect: total,
        }),
        None => fallback,
    }
}

pub async fn multimodal_embedding(
    call: &MultimodalCall<'_>,
    catalog: &ModelCatalog,
    cache: &CredentialCache,
    clients: &MultimodalClients<'_>,
) -> Result<MultimodalEmbeddingAnswer, VertexEmbeddingCallError> {
    let prepared = prepare(call, catalog, cache).await?;
    if call.aembedding {
        // LiteLLM's multimodal handoff does forward the client the caller supplied, unlike the text handler
        return async_multimodal_embedding(
            &prepared,
            call.model,
            call.timeout,
            clients.caller,
            clients.pooled,
        )
        .await;
    }
    sync_embedding(&prepared, call.model, call.timeout, clients).await
}

async fn prepare(
    call: &MultimodalCall<'_>,
    catalog: &ModelCatalog,
    cache: &CredentialCache,
) -> Result<MultimodalRequest, VertexEmbeddingCallError> {
    let access = ensure_access_token(
        call.provider,
        cache.get_access_token(call.credentials, call.project),
    )
    .await
    .map_err(|source| VertexEmbeddingCallError::Credentials { source })?;
    let target = target::embedding_target(
        catalog,
        &TargetRequest {
            provider: call.provider,
            model: call.model,
            auth_token: Some(&access.token),
            gemini_api_key: call.gemini_api_key,
            project: Some(access.project.as_str()),
            location: call.location,
            api_base: call.api_base,
            use_psc_endpoint_format: false,
        },
    )
    .map_err(|source| VertexEmbeddingCallError::Target { source })?;
    let headers =
        target::embedding_headers(target.auth.as_ref(), call.headers, HeaderMode::Multimodal);
    let payload = transform_request(call.input, call.optional_params)?;
    let body = serde_json::to_vec(&payload)
        .map_err(|source| VertexEmbeddingCallError::Encode { source })?;
    Ok(MultimodalRequest {
        url: target.url,
        headers,
        body,
        payload,
    })
}

async fn sync_embedding(
    request: &MultimodalRequest,
    model: &str,
    timeout: Option<Duration>,
    clients: &MultimodalClients<'_>,
) -> Result<MultimodalEmbeddingAnswer, VertexEmbeddingCallError> {
    let (post, client_timeout) = multimodal_post(
        timeout,
        ClientTimeout::Explicit(HANDLER_DEFAULT_TIMEOUT),
        clients.pooled,
        clients.caller,
    );
    post_prepared(request, model, post, client_timeout).await
}

pub async fn async_multimodal_embedding(
    request: &MultimodalRequest,
    model: &str,
    timeout: Option<Duration>,
    caller: Option<&dyn EmbeddingPost>,
    pooled: &dyn EmbeddingPost,
) -> Result<MultimodalEmbeddingAnswer, VertexEmbeddingCallError> {
    let (post, client_timeout) =
        multimodal_post(timeout, ClientTimeout::HandlerDefault, pooled, caller);
    post_prepared(request, model, post, client_timeout).await
}

// A supplied client keeps its own timeouts; the call's timeout only reaches a client LiteLLM builds
fn multimodal_post<'client>(
    timeout: Option<Duration>,
    default: ClientTimeout,
    pooled: &'client dyn EmbeddingPost,
    caller: Option<&'client dyn EmbeddingPost>,
) -> (&'client dyn EmbeddingPost, ClientTimeout) {
    caller.map_or_else(
        || (pooled, multimodal_timeout(timeout, default)),
        |supplied| (supplied, ClientTimeout::HandlerDefault),
    )
}

// Both branches post the same request, and httpx's raise_for_status keeps the status a sync reply carries too
async fn post_prepared(
    request: &MultimodalRequest,
    model: &str,
    post: &dyn EmbeddingPost,
    client_timeout: ClientTimeout,
) -> Result<MultimodalEmbeddingAnswer, VertexEmbeddingCallError> {
    logging::request(&request.url, model, request.body.len());
    let reply = post
        .post(PostRequest {
            url: &request.url,
            headers: &request.headers,
            body: &request.body,
            timeout: client_timeout,
        })
        .await?;
    logging::answer(model, &reply);
    if !(200..300).contains(&reply.status) {
        return Err(VertexEmbeddingCallError::Status {
            status: reply.status,
            body: reply.body,
        });
    }
    // A 2xx that is not 200 still reaches the transformation, which refuses it with its own 500
    transform_response(reply.status, &reply.body, &request.payload, model)
        .map_err(VertexEmbeddingCallError::Multimodal)
}
