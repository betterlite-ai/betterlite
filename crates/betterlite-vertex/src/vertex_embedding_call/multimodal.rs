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
use super::target::{self, TargetRequest};

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
    let headers = target::headers(
        target.auth.as_ref(),
        target::MULTIMODAL_CONTENT_TYPE,
        call.headers,
    );
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
    let post = clients.caller.unwrap_or(clients.pooled);
    logging::request(&request.url, model, request.body.len());
    // The sync branch never checks the status itself, so every transport failure is its generic 500
    let reply = post
        .post(PostRequest {
            url: &request.url,
            headers: &request.headers,
            body: &request.body,
            timeout: multimodal_timeout(timeout, ClientTimeout::Explicit(HANDLER_DEFAULT_TIMEOUT)),
        })
        .await
        .map_err(|source| VertexEmbeddingCallError::Transport { source })?;
    logging::answer(model, &reply);
    transform_response(reply.status, &reply.body, &request.payload, model)
        .map_err(VertexEmbeddingCallError::Multimodal)
}

pub async fn async_multimodal_embedding(
    request: &MultimodalRequest,
    model: &str,
    timeout: Option<Duration>,
    caller: Option<&dyn EmbeddingPost>,
    pooled: &dyn EmbeddingPost,
) -> Result<MultimodalEmbeddingAnswer, VertexEmbeddingCallError> {
    let post = caller.unwrap_or(pooled);
    logging::request(&request.url, model, request.body.len());
    let reply = post
        .post(PostRequest {
            url: &request.url,
            headers: &request.headers,
            body: &request.body,
            timeout: multimodal_timeout(timeout, ClientTimeout::HandlerDefault),
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
