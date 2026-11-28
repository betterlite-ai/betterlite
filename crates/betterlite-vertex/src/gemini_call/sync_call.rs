use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_openai::aleph_alpha::lines::decode_text;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use secrecy::SecretString;
use url::Url;
use uuid::Uuid;

use crate::cache_lookup::transport::CacheTransport;
use crate::gemini_http::call::Clients;
use crate::gemini_http::error::{GeminiHttpError, get_error_class};
use crate::gemini_http::headers::request_headers;
use crate::gemini_http::post::{GeminiPost, PostRequest};
use crate::gemini_http::stream::GeminiHttpStream;
use crate::gemini_http::stream::StreamSetup;
use crate::gemini_request::orchestration::transform_request_body;
use crate::gemini_response::complete::{CompleteRequest, RawResponse, transform_response};
use crate::gemini_stream::StreamOptions;
use crate::token_refresh::ensure_access_token;
use crate::vertex_embedding_call::post::ClientTimeout;
use betterlite_core::api::vector_store_http_write::service::ClientKind;

use super::body::body_request;
use super::error::GeminiCallError;
use super::selection::sync_client_timeout;
use super::target::resolve_target;
use super::{CallDeps, GeminiCall, GeminiReply};

struct Prepared {
    url: String,
    headers: BTreeMap<String, SecretString>,
    body: Vec<u8>,
}

// LiteLLM's synchronous branch: authenticate, resolve the target, build the body, then run the call
pub(super) async fn completion<Cache, Fetch, Log, Console>(
    call: &GeminiCall<'_>,
    optional_params: &Object,
    deps: &CallDeps<'_, Cache, Fetch, Log, Console>,
    stream: bool,
    identifiers: Box<dyn FnMut() -> Uuid + Send>,
) -> Result<GeminiReply, GeminiCallError>
where
    Cache: CacheTransport,
    Fetch: RemoteMediaFetcher + Sync,
    Log: LoggerSink + Sync,
    Console: ConsoleSink + Sync,
{
    let prepared = prepare(call, optional_params, deps, stream).await?;
    log_request(&prepared.url, call.model, prepared.body.len());
    if stream {
        let setup = StreamSetup {
            url: prepared.url,
            headers: prepared.headers,
            body: prepared.body,
            model: call.model.to_owned(),
            supplied: stream_client(deps.clients),
            pooled: Arc::clone(&deps.clients.pooled),
            ids: identifiers,
        };
        let options = StreamOptions {
            now: call.now.as_second(),
            call_mode: call.call_mode,
        };
        return Ok(GeminiReply::Stream(Box::new(GeminiHttpStream::new(
            setup, options,
        ))));
    }
    let (post, timeout) = post_client(deps.clients, call.timeout);
    complete(call, post, timeout, &prepared, identifiers).await
}

async fn prepare<Cache, Fetch, Log, Console>(
    call: &GeminiCall<'_>,
    optional_params: &Object,
    deps: &CallDeps<'_, Cache, Fetch, Log, Console>,
    stream: bool,
) -> Result<Prepared, GeminiCallError>
where
    Cache: CacheTransport,
    Fetch: RemoteMediaFetcher + Sync,
    Log: LoggerSink + Sync,
    Console: ConsoleSink + Sync,
{
    let lookup = deps
        .llm
        .cache
        .get_access_token(call.credentials, call.project);
    let access = ensure_access_token(call.provider, lookup)
        .await
        .map_err(|source| GeminiHttpError::Credentials { source })?;
    let target = resolve_target(call, optional_params, deps.catalog, &access, stream)?;
    let headers = request_headers(target.auth.as_ref(), call.extra_headers);
    let request = body_request(call, optional_params, deps, &access);
    let transformed = transform_request_body(deps.cache, request, deps.hooks).await?;
    let body = serde_json::to_vec(&transformed.body)
        .map_err(|source| GeminiHttpError::Encode { source })?;
    Ok(Prepared {
        url: target.url,
        headers,
        body,
    })
}

// LiteLLM's isinstance(client, HTTPHandler): a supplied sync client keeps its own timeouts and is never replaced
fn post_client(
    clients: &Clients,
    timeout: Option<Duration>,
) -> (Arc<dyn GeminiPost>, ClientTimeout) {
    match &clients.supplied {
        Some(supplied) if supplied.kind == ClientKind::Sync => (
            Arc::clone(&supplied.transport),
            ClientTimeout::HandlerDefault,
        ),
        Some(_) | None => (Arc::clone(&clients.pooled), sync_client_timeout(timeout)),
    }
}

// LiteLLM's sync streaming branch hands make_sync_call the caller's sync handler only
fn stream_client(clients: &Clients) -> Option<Arc<dyn GeminiPost>> {
    match &clients.supplied {
        Some(supplied) if supplied.kind == ClientKind::Sync => {
            Some(Arc::clone(&supplied.transport))
        }
        Some(_) | None => None,
    }
}

async fn complete(
    call: &GeminiCall<'_>,
    post: Arc<dyn GeminiPost>,
    timeout: ClientTimeout,
    prepared: &Prepared,
    mut identifiers: Box<dyn FnMut() -> Uuid + Send>,
) -> Result<GeminiReply, GeminiCallError> {
    let reply = post
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
        let error = get_error_class(message, reply.status, &reply.headers);
        return Err(GeminiCallError::Http(GeminiHttpError::Provider(error)));
    }
    let response = transform_response(
        &RawResponse {
            headers: &reply.headers,
            body: &reply.body,
        },
        &CompleteRequest {
            model: call.model,
            now: call.now.as_second(),
            call_mode: call.call_mode,
        },
        identifiers.as_mut(),
    )
    .map_err(GeminiHttpError::Provider)?;
    Ok(GeminiReply::Complete(Box::new(response)))
}

// B1316's pre_call facts, cut to values that carry no header or credential
fn log_request(url: &str, model: &str, body_bytes: usize) {
    let host = Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned));
    tracing::debug!(
        model,
        host = host.as_deref().unwrap_or_default(),
        body_bytes,
        "sending the Gemini request"
    );
}
