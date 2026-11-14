use std::sync::atomic::{AtomicU8, Ordering};

use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_prompts::media::inline::{RemoteMediaFetcher, remote_urls_to_inline};
use betterlite_prompts::message_content::Message;

use crate::cache_create::orchestration::{CacheCreateRequest, check_and_create_cache};
use crate::cache_lookup::transport::CacheTransport;
use crate::gemini_history::content::{CONTENT_TYPE, FORMAT, MIME_TYPE};
use crate::gemini_history::error::HistoryError;
use crate::gemini_history::media_source::text_of;
use crate::gemini_media::error::{MediaError, MediaField};
use crate::gemini_media::mime::MimeHints;
use crate::gemini_media::policy::{GoogleProvider, inlines_remote};
use crate::gemini_request::body::Transformed;
use crate::gemini_request::error::{BodyError, RequestError};
use crate::model_policy::capabilities::VertexCapabilityProvider;

use super::{CallRequest, body_from, cache_options, capability_provider, restore_cache_options};

// LiteLLM's async entry point downloads remote media before the body; betterlite classifies and refuses
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
    let mut optional_params = request.optional_params.clone();
    let (cached_content, mut options) = cache_options(&mut optional_params)?;
    let cache_request = CacheCreateRequest {
        cached_content: cached_content.as_deref(),
        ..request.cache
    };
    let outcome = check_and_create_cache(transport, &cache_request, &mut options, hooks).await?;
    restore_cache_options(&mut optional_params, &options)?;
    reject_inlined_media(
        capability_provider(cache_request.provider),
        &outcome.messages,
    )?;
    body_from(&cache_request, &request, optional_params, &outcome).await
}

// LiteLLM downloads every remote media its predicate accepts; betterlite refuses the request without fetching
fn reject_inlined_media(
    provider: VertexCapabilityProvider,
    messages: &[Message],
) -> Result<(), RequestError> {
    let google = match provider {
        VertexCapabilityProvider::Gemini => GoogleProvider::AiStudio,
        VertexCapabilityProvider::VertexAi | VertexCapabilityProvider::VertexAiBeta => {
            GoogleProvider::Vertex
        }
    };
    let first = AtomicU8::new(0);
    let urls = remote_urls_to_inline(messages, &|media| {
        let hints = MimeHints {
            format: text_of(&media.fields, FORMAT),
            mime_type: text_of(&media.fields, MIME_TYPE),
            content_type: text_of(&media.fields, CONTENT_TYPE),
        };
        let inlines = inlines_remote(google, &media.url, &hints);
        if inlines && first.load(Ordering::Relaxed) == 0 {
            first.store(
                if media.part_type == "file" { 2 } else { 1 },
                Ordering::Relaxed,
            );
        }
        inlines
    })?;
    if urls.is_empty() {
        return Ok(());
    }
    let field = if first.load(Ordering::Relaxed) == 2 {
        MediaField::File
    } else {
        MediaField::ImageUrl
    };
    let error = BodyError::History(HistoryError::Media(MediaError::RemoteMediaRequired(field)));
    Err(RequestError::Body(error))
}
