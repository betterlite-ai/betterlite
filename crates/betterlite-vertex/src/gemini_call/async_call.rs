use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use uuid::Uuid;

use crate::cache_lookup::transport::CacheTransport;
use crate::gemini_http::call::{GeminiCall as HttpCall, async_completion, async_streaming};
use crate::gemini_http::error::GeminiHttpError;
use crate::gemini_request::orchestration::async_media::transform_request_body_async;
use crate::token_refresh::ensure_access_token;

use super::body::body_request;
use super::error::GeminiCallError;
use super::target::resolve_target;
use super::{CallDeps, GeminiCall, GeminiReply, GeminiStream};

// LiteLLM's acompletion branch: B1316 builds the URL, the headers and the request and then runs it
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
    let lookup = deps
        .llm
        .refresh
        .get_access_token_async(call.credentials, call.project);
    let access = ensure_access_token(call.provider, lookup)
        .await
        .map_err(|source| GeminiHttpError::Credentials { source })?;
    // LiteLLM resolves the URL before the async body transform, so a bad target fails before any cache or media request
    resolve_target(call, optional_params, deps.catalog, &access, stream)?;
    let request = body_request(call, optional_params, deps, &access);
    let transformed = transform_request_body_async(deps.cache, request, deps.hooks).await?;
    let http = HttpCall {
        provider: call.provider,
        model: call.model,
        body: &transformed.body,
        optional_params,
        api_base: call.api_base,
        gemini_api_key: call.gemini_api_key,
        project: call.project,
        location: call.location,
        credentials: call.credentials,
        extra_headers: call.extra_headers,
        timeout: call.timeout,
        now: call.now.as_second(),
        call_mode: call.call_mode,
    };
    if stream {
        let opened =
            async_streaming(&http, deps.catalog, deps.llm, deps.clients, identifiers).await?;
        return Ok(GeminiReply::Stream(Box::new(GeminiStream::Async(opened))));
    }
    let response =
        async_completion(&http, deps.catalog, deps.llm, deps.clients, identifiers).await?;
    Ok(GeminiReply::Complete(Box::new(response)))
}
