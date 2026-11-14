use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse};
use secrecy::SecretString;

use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::token_cache::CredentialCache;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::token_refresh::CredentialRefresh;
use crate::token_refresh::ensure_access_token;
use crate::vertex_text_embeddings::error::VertexTextEmbeddingError;
use crate::vertex_text_embeddings::request::transform_request;
use crate::vertex_text_embeddings::response::transform_response;

use super::decode::decode_vertex_embedding;
use super::error::VertexEmbeddingCallError;
use super::logging;
use super::post::{ClientTimeout, EmbeddingPost, PostReply, PostRequest, Timeout};
use super::target::{self, TargetRequest};

pub struct TextCall<'call> {
    pub provider: VertexCapabilityProvider,
    pub model: &'call str,
    pub input: &'call EmbeddingInput,
    pub optional_params: &'call Json,
    pub litellm_params: Option<&'call Json>,
    pub api_base: Option<&'call str>,
    pub gemini_api_key: Option<&'call SecretString>,
    pub project: Option<&'call str>,
    pub location: Option<&'call str>,
    pub credentials: Option<&'call ConfiguredCredentials>,
    pub extra_headers: &'call BTreeMap<String, String>,
    pub timeout: Option<Duration>,
    pub aembedding: bool,
}

pub struct TextAuth<'auth> {
    pub cache: &'auth CredentialCache,
    pub refresh: &'auth CredentialRefresh,
}

pub struct TextClients<'clients> {
    pub pooled: &'clients dyn EmbeddingPost,
    pub caller: Option<&'clients dyn EmbeddingPost>,
}

struct Prepared {
    url: String,
    headers: BTreeMap<String, SecretString>,
    body: Vec<u8>,
}

// LiteLLM applies the timeout only when it is truthy, so an explicit zero keeps the handler's own timeout
#[must_use]
pub(super) const fn text_client_timeout(timeout: Option<Duration>) -> ClientTimeout {
    match timeout {
        Some(total) if !total.is_zero() => ClientTimeout::Explicit(Timeout {
            total,
            connect: total,
        }),
        Some(_) | None => ClientTimeout::HandlerDefault,
    }
}

fn psc_endpoint_format(optional_params: &Json) -> Result<bool, VertexEmbeddingCallError> {
    let Json::Object(params) = optional_params else {
        return Err(VertexEmbeddingCallError::TextRequest {
            source: VertexTextEmbeddingError::OptionalParams,
        });
    };
    match params.get("use_psc_endpoint_format") {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(flag)) => Ok(*flag),
        Some(_) => Err(VertexEmbeddingCallError::Flag {
            field: "use_psc_endpoint_format",
        }),
    }
}

pub async fn text_embedding(
    call: &TextCall<'_>,
    catalog: &ModelCatalog,
    auth: &TextAuth<'_>,
    clients: &TextClients<'_>,
) -> Result<EmbeddingResponse, VertexEmbeddingCallError> {
    if call.aembedding {
        // LiteLLM's outer handoff never passes the client it was given, so the caller's handler cannot be posted through
        return async_text_embedding(call, catalog, auth.refresh, clients.pooled, None).await;
    }
    let post = clients.caller.unwrap_or(clients.pooled);
    let lookup = auth.cache.get_access_token(call.credentials, call.project);
    run(call, catalog, lookup, post).await
}

pub async fn async_text_embedding(
    call: &TextCall<'_>,
    catalog: &ModelCatalog,
    refresh: &CredentialRefresh,
    pooled: &dyn EmbeddingPost,
    caller: Option<&dyn EmbeddingPost>,
) -> Result<EmbeddingResponse, VertexEmbeddingCallError> {
    let post = caller.unwrap_or(pooled);
    run(
        call,
        catalog,
        refresh.get_access_token_async(call.credentials, call.project),
        post,
    )
    .await
}

async fn run<Lookup>(
    call: &TextCall<'_>,
    catalog: &ModelCatalog,
    lookup: Lookup,
    post: &dyn EmbeddingPost,
) -> Result<EmbeddingResponse, VertexEmbeddingCallError>
where
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let prepared = prepare(call, catalog, lookup).await?;
    send(
        &prepared,
        call.model,
        text_client_timeout(call.timeout),
        post,
    )
    .await
}

async fn prepare<Lookup>(
    call: &TextCall<'_>,
    catalog: &ModelCatalog,
    lookup: Lookup,
) -> Result<Prepared, VertexEmbeddingCallError>
where
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let access = ensure_access_token(call.provider, lookup)
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
            use_psc_endpoint_format: psc_endpoint_format(call.optional_params)?,
        },
    )
    .map_err(|source| VertexEmbeddingCallError::Target { source })?;
    let headers = target::headers(
        target.auth.as_ref(),
        target::TEXT_CONTENT_TYPE,
        call.extra_headers,
    );
    let request = transform_request(
        call.input,
        call.optional_params,
        call.model,
        call.litellm_params,
    )
    .map_err(|source| VertexEmbeddingCallError::TextRequest { source })?;
    let body = serde_json::to_vec(&request)
        .map_err(|source| VertexEmbeddingCallError::Encode { source })?;
    Ok(Prepared {
        url: target.url,
        headers,
        body,
    })
}

async fn send(
    prepared: &Prepared,
    model: &str,
    timeout: ClientTimeout,
    post: &dyn EmbeddingPost,
) -> Result<EmbeddingResponse, VertexEmbeddingCallError> {
    logging::request(&prepared.url, model, prepared.body.len());
    let reply = post
        .post(PostRequest {
            url: &prepared.url,
            headers: &prepared.headers,
            body: &prepared.body,
            timeout,
        })
        .await?;
    logging::answer(model, &reply);
    embedded(reply, model)
}

fn embedded(reply: PostReply, model: &str) -> Result<EmbeddingResponse, VertexEmbeddingCallError> {
    if !(200..300).contains(&reply.status) {
        return Err(VertexEmbeddingCallError::Status {
            status: reply.status,
            body: reply.body,
        });
    }
    let decoded = decode_vertex_embedding(&reply.body)?;
    transform_response(&decoded, model)
        .map_err(|source| VertexEmbeddingCallError::TextAnswer { source })
}
