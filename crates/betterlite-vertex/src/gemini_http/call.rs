use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_core::api::vector_store_http_write::service::ClientKind;
use betterlite_models::catalog::ModelCatalog;
use betterlite_openai::aleph_alpha::lines::decode_text;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use uuid::Uuid;

use crate::endpoint_target::target::{TargetProvider, TokenUrlInput, get_token_and_url};
use crate::gemini_response::complete::{
    CompleteRequest, CompleteResponse, RawResponse, transform_response,
};
use crate::gemini_response_parts::projection::CallMode;
use crate::gemini_stream::StreamOptions;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::model_urls::{ApiVersion, UrlMode};
use crate::token_cache::CredentialCache;
use crate::token_refresh::{CredentialRefresh, ensure_access_token};
use crate::vertex_embedding_call::post::{ClientTimeout, Timeout};

use super::error::{GeminiHttpError, get_error_class};
use super::headers::{psc_endpoint_format, request_headers};
use super::logging;
use super::post::{GeminiPost, PostRequest};
use super::stream::{GeminiHttpStream, StreamSetup};

pub struct SuppliedClient {
    pub kind: ClientKind,
    pub transport: Arc<dyn GeminiPost>,
}

pub struct Clients {
    pub supplied: Option<SuppliedClient>,
    pub pooled: Arc<dyn GeminiPost>,
}

// VertexBase's constructor: one credential cache beside its refresh bookkeeping, built once when the client is wired
pub struct VertexLlm {
    pub cache: Arc<CredentialCache>,
    pub refresh: Arc<CredentialRefresh>,
}

pub struct GeminiCall<'call> {
    pub provider: VertexCapabilityProvider,
    pub model: &'call str,
    pub body: &'call Object,
    pub optional_params: &'call Object,
    pub api_base: Option<&'call str>,
    pub gemini_api_key: Option<&'call SecretString>,
    pub project: Option<&'call str>,
    pub location: Option<&'call str>,
    pub credentials: Option<&'call ConfiguredCredentials>,
    pub extra_headers: &'call BTreeMap<String, String>,
    pub timeout: Option<Duration>,
    pub now: i64,
    pub call_mode: CallMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CallKind {
    Complete,
    Stream,
}

struct Prepared {
    url: String,
    headers: BTreeMap<String, SecretString>,
    body: Vec<u8>,
}

// LiteLLM's `if timeout:`: only a truthy timeout reaches the async client it builds
#[must_use]
const fn completion_timeout(timeout: Option<Duration>) -> ClientTimeout {
    match timeout {
        Some(total) if !total.is_zero() => ClientTimeout::Explicit(Timeout {
            total,
            connect: total,
        }),
        Some(_) | None => ClientTimeout::HandlerDefault,
    }
}

// LiteLLM's isinstance(client, AsyncHTTPHandler): a supplied client keeps its own timeout, any other one is replaced
fn completion_client(
    clients: &Clients,
    timeout: Option<Duration>,
) -> (Arc<dyn GeminiPost>, ClientTimeout) {
    match &clients.supplied {
        Some(supplied) if supplied.kind == ClientKind::Asynchronous => (
            Arc::clone(&supplied.transport),
            ClientTimeout::HandlerDefault,
        ),
        Some(_) | None => (Arc::clone(&clients.pooled), completion_timeout(timeout)),
    }
}

// LiteLLM hands the streamed call the caller's async handler only, and no timeout at all
fn streaming_client(clients: &Clients) -> Option<Arc<dyn GeminiPost>> {
    match &clients.supplied {
        Some(supplied) if supplied.kind == ClientKind::Asynchronous => {
            Some(Arc::clone(&supplied.transport))
        }
        Some(_) | None => None,
    }
}

const fn target_provider(provider: VertexCapabilityProvider) -> TargetProvider {
    match provider {
        VertexCapabilityProvider::VertexAi | VertexCapabilityProvider::VertexAiBeta => {
            TargetProvider::Vertex
        }
        VertexCapabilityProvider::Gemini => TargetProvider::Gemini,
    }
}

async fn prepare(
    call: &GeminiCall<'_>,
    catalog: &ModelCatalog,
    llm: &VertexLlm,
    kind: CallKind,
) -> Result<Prepared, GeminiHttpError> {
    let lookup = llm
        .refresh
        .get_access_token_async(call.credentials, call.project);
    let access = ensure_access_token(call.provider, lookup)
        .await
        .map_err(|source| GeminiHttpError::Credentials { source })?;
    let target = get_token_and_url(
        catalog,
        &TokenUrlInput {
            provider: target_provider(call.provider),
            model: call.model,
            auth_token: Some(&access.token),
            gemini_api_key: call.gemini_api_key,
            project: Some(access.project.as_str()),
            location: call.location,
            stream: kind == CallKind::Stream,
            api_base: call.api_base,
            api_version: ApiVersion::V1,
            mode: UrlMode::Chat,
            use_psc_endpoint_format: psc_endpoint_format(call.optional_params)?,
        },
    )
    .map_err(|source| GeminiHttpError::Target { source })?;
    let headers = request_headers(target.auth.as_ref(), call.extra_headers);
    let body =
        serde_json::to_vec(call.body).map_err(|source| GeminiHttpError::Encode { source })?;
    Ok(Prepared {
        url: target.url,
        headers,
        body,
    })
}

pub async fn async_completion(
    call: &GeminiCall<'_>,
    catalog: &ModelCatalog,
    llm: &VertexLlm,
    clients: &Clients,
    mut ids: Box<dyn FnMut() -> Uuid + Send>,
) -> Result<CompleteResponse, GeminiHttpError> {
    let prepared = prepare(call, catalog, llm, CallKind::Complete).await?;
    let (client, timeout) = completion_client(clients, call.timeout);
    logging::request(&prepared.url, call.model, prepared.body.len());
    let reply = client
        .post(PostRequest {
            url: &prepared.url,
            headers: &prepared.headers,
            body: &prepared.body,
            timeout,
        })
        .await
        .map_err(GeminiHttpError::transport)?;
    tracing::debug!(
        model = call.model,
        status = reply.status,
        "received the Gemini answer"
    );
    // LiteLLM's raise_for_status rejects every non-2xx reply, an unfollowed redirect included
    if !(200..300).contains(&reply.status) {
        let message = decode_text(&reply.body);
        return Err(GeminiHttpError::Provider(get_error_class(
            message,
            reply.status,
            &reply.headers,
        )));
    }
    transform_response(
        &RawResponse {
            headers: &reply.headers,
            body: &reply.body,
        },
        &CompleteRequest {
            model: call.model,
            now: call.now,
            call_mode: call.call_mode,
        },
        ids.as_mut(),
    )
    .map_err(GeminiHttpError::Provider)
}

pub async fn async_streaming(
    call: &GeminiCall<'_>,
    catalog: &ModelCatalog,
    llm: &VertexLlm,
    clients: &Clients,
    ids: Box<dyn FnMut() -> Uuid + Send>,
) -> Result<GeminiHttpStream, GeminiHttpError> {
    let prepared = prepare(call, catalog, llm, CallKind::Stream).await?;
    logging::request(&prepared.url, call.model, prepared.body.len());
    Ok(GeminiHttpStream::new(
        StreamSetup {
            url: prepared.url,
            headers: prepared.headers,
            body: prepared.body,
            model: call.model.to_owned(),
            supplied: streaming_client(clients),
            pooled: Arc::clone(&clients.pooled),
            ids,
        },
        StreamOptions {
            now: call.now,
            call_mode: call.call_mode,
        },
    ))
}
