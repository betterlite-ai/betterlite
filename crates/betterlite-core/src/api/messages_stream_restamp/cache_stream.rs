use std::convert::Infallible;
use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::handler::HandlerError;
use betterlite_cache::handler::call_types::should_store_result_in_cache;
use betterlite_cache::handler::kwargs::CACHE_KEY_KWARG;
use betterlite_cache::handler::state::OriginalFunction;
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_cache::write_completion::tracker::{CacheWriteFuture, CacheWriteTracker};
use betterlite_context::call_context::{CallContext, CallState};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::messages_sse_events::chunk::ChunkFlag;
use crate::api::messages_sse_events::decode_messages_sse_chunk;
use crate::api::messages_sse_events::stream::MessagesProviderStream;

use super::CACHED_STREAM_EVENTS_KEY;

// the cache write failure LiteLLM's writer logs; the only consumer is that log line, which names no fields
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct CacheWriteFailure {
    pub message: String,
}

// The cache LiteLLM's writer awaits: cache.async_add_cache(payload, dynamic_cache_object=dual_cache, **request_kwargs)
#[async_trait]
pub trait MessagesStreamCacheTarget: Send + Sync {
    async fn add_cached_stream(
        &self,
        payload: Json,
        request_kwargs: KeyObject,
    ) -> Result<(), CacheWriteFailure>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessagesStreamCacheSettings {
    pub original_function: OriginalFunction,
    pub supported_call_types: Option<Vec<CachingSupportedCallType>>,
    pub request_kwargs: KeyObject,
    pub preset_cache_key: Option<String>,
}

// everything the write path is injected with: the configured cache, B4304's tracker, the call it writes under, and B1087's limits
pub struct MessagesStreamCacheDeps {
    pub cache: Option<Arc<dyn MessagesStreamCacheTarget>>,
    pub tracker: Arc<CacheWriteTracker>,
    pub call: CallContext<'static>,
    pub limits: ApiLimits,
}

#[derive(Debug, Error)]
pub enum MessagesStreamCacheError<StreamError = Infallible>
where
    StreamError: Error + 'static,
{
    #[error(transparent)]
    Stream(StreamError),
    #[error("the cache store decision failed")]
    Store(#[from] HandlerError),
    #[error(transparent)]
    Boundary(#[from] ApiLimitError),
}

// LiteLLM's AnthropicMessagesStreamCacheWriter: collect what the client saw, then cache the joined events once the stream ends
pub struct MessagesStreamCacheWriter<S> {
    stream: S,
    settings: MessagesStreamCacheSettings,
    cache: Option<Arc<dyn MessagesStreamCacheTarget>>,
    tracker: Arc<CacheWriteTracker>,
    call: CallContext<'static>,
    limits: ApiLimits,
    collected: Vec<Vec<u8>>,
    collected_bytes: usize,
    collection_stopped: bool,
    persisted: bool,
}

impl<S> MessagesStreamCacheWriter<S> {
    #[must_use]
    pub fn new(
        stream: S,
        settings: MessagesStreamCacheSettings,
        deps: MessagesStreamCacheDeps,
    ) -> Self {
        Self {
            stream,
            settings,
            cache: deps.cache,
            tracker: deps.tracker,
            call: deps.call,
            limits: deps.limits,
            collected: Vec::new(),
            collected_bytes: 0,
            collection_stopped: false,
            persisted: false,
        }
    }
}

#[async_trait]
impl<S> MessagesProviderStream for MessagesStreamCacheWriter<S>
where
    S: MessagesProviderStream + Send,
    S::Error: Error + 'static,
{
    type Error = MessagesStreamCacheError<S::Error>;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Self::Error> {
        match self.stream.next_chunk().await {
            Ok(Some(chunk)) => {
                self.collect(&chunk);
                Ok(Some(chunk))
            }
            Ok(None) => {
                self.persist()?;
                Ok(None)
            }
            Err(source) => Err(MessagesStreamCacheError::Stream(source)),
        }
    }

    async fn aclose(&mut self) -> Option<Result<(), Self::Error>> {
        self.stream
            .aclose()
            .await
            .map(|result| result.map_err(MessagesStreamCacheError::Stream))
    }

    fn buffered_provider_output(&self) -> Option<Json> {
        self.has_buffered_provider_output()
            .then_some(Json::Bool(true))
    }
}

impl<S> MessagesStreamCacheWriter<S>
where
    S: MessagesProviderStream,
    S::Error: Error + 'static,
{
    // LiteLLM's writer property: only an inner stream reporting the literal True buffers provider output
    #[must_use]
    pub fn has_buffered_provider_output(&self) -> bool {
        matches!(
            self.stream.buffered_provider_output(),
            Some(Json::Bool(true))
        )
    }

    // A stream bigger than the resolved limits is not cached, so the collected copy stays bounded
    fn collect(&mut self, chunk: &[u8]) {
        if self.collection_stopped {
            return;
        }
        let items = self.collected.len().saturating_add(1);
        let bytes = self.collected_bytes.saturating_add(chunk.len());
        if items > self.limits.max_items || bytes > self.limits.max_json_bytes {
            self.collection_stopped = true;
            self.collected = Vec::new();
            self.collected_bytes = 0;
            return;
        }
        self.collected.push(chunk.to_vec());
        self.collected_bytes = bytes;
    }

    fn persist(&mut self) -> Result<(), MessagesStreamCacheError<S::Error>> {
        let Some(target) = self.cache.as_ref().map(Arc::clone) else {
            return Ok(());
        };
        if self.persisted || self.collection_stopped {
            return Ok(());
        }
        let collected = self.collected.concat();
        let decoded = decode_messages_sse_chunk(&collected, &self.scan_limits())?;
        let complete = decoded.flags.contains(&ChunkFlag::MessageStop)
            && !decoded.flags.contains(&ChunkFlag::ProviderError);
        if !complete {
            return Ok(());
        }
        self.persisted = true;
        let store = should_store_result_in_cache(
            &self.settings.original_function.name,
            self.settings.supported_call_types.as_deref(),
            &self.settings.request_kwargs,
        )?;
        if !store {
            return Ok(());
        }
        self.start_write(target, &collected);
        Ok(())
    }

    fn start_write(&self, target: Arc<dyn MessagesStreamCacheTarget>, collected: &[u8]) {
        match str::from_utf8(collected) {
            Ok(text) => self.spawn_write(target, text),
            Err(error) => tracing::error!("Anthropic Messages stream cache write failed: {error}"),
        }
    }

    fn spawn_write(&self, target: Arc<dyn MessagesStreamCacheTarget>, text: &str) {
        let events: Vec<Json> = split_sse_events(text).into_iter().map(Json::Text).collect();
        let payload = Json::Object(IndexMap::from([(
            CACHED_STREAM_EVENTS_KEY.to_owned(),
            Json::List(events),
        )]));
        let request_kwargs = self.request_kwargs();
        let factory = move |_call: CallState| -> CacheWriteFuture {
            let cache_target = Arc::clone(&target);
            let cached_payload = payload.clone();
            let cached_request_kwargs = request_kwargs.clone();
            Box::pin(async move {
                let outcome = cache_target
                    .add_cached_stream(cached_payload, cached_request_kwargs)
                    .await;
                log_write_failure(outcome);
                Ok(())
            })
        };
        let _write = self.tracker.spawn(&self.call, factory);
    }

    fn request_kwargs(&self) -> KeyObject {
        let mut request_kwargs = self.settings.request_kwargs.clone();
        if let Some(preset) = &self.settings.preset_cache_key {
            request_kwargs.insert(CACHE_KEY_KWARG.to_owned(), KeyValue::String(preset.clone()));
        }
        request_kwargs
    }

    // the joined stream is scanned as one event, so the event limit is the collected payload's limit
    const fn scan_limits(&self) -> ApiLimits {
        ApiLimits {
            max_event_bytes: self.limits.max_json_bytes,
            ..self.limits
        }
    }
}

// LiteLLM logs a failed stream cache write and keeps serving the stream
fn log_write_failure(outcome: Result<(), CacheWriteFailure>) {
    if let Err(failure) = outcome {
        tracing::error!("Anthropic Messages stream cache write failed: {failure}");
    }
}

// LiteLLM's _split_sse_events: split after every "\n\n" and drop the empty pieces
fn split_sse_events(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut events = Vec::new();
    let mut start = 0_usize;
    for index in 0..bytes.len() {
        if bytes.get(index.saturating_sub(2)..index) == Some(b"\n\n".as_slice()) {
            if let Some(event) = text.get(start..index) {
                events.push(event.to_owned());
            }
            start = index;
        }
    }
    if let Some(event) = text.get(start..)
        && !event.is_empty()
    {
        events.push(event.to_owned());
    }
    events
}
