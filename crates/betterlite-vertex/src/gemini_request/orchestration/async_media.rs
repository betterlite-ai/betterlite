use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_prompts::media::inline::RemoteMediaFetcher;

use crate::cache_lookup::transport::CacheTransport;
use crate::gemini_request::body::Transformed;
use crate::gemini_request::error::RequestError;
use crate::gemini_request::media_prepare::FetchMode;

use super::CallRequest;

pub async fn transform_request_body_async<T, F, L, C>(
    transport: &T,
    request: CallRequest<'_, F>,
    hooks: &VerboseHooks<L, C>,
) -> Result<Transformed, RequestError>
where
    T: CacheTransport,
    F: RemoteMediaFetcher + Sync,
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    // LiteLLM's async entry pre-inlines the whole message list in one batch, as B5083's fetch_all does
    super::transformed(transport, request, hooks, FetchMode::Concurrent).await
}
