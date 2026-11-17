use std::convert::Infallible;
use std::str::Utf8Error;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::replay::CachedDict;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use thiserror::Error;

use crate::api::messages_sse_events::stream::MessagesProviderStream;
use crate::api::messages_stream_logging::MessagesStreamLogging;
use crate::api::messages_stream_logging::seams::StreamSeams;

use super::CACHED_STREAM_EVENTS_KEY;

const CACHE_HIT_KEY: &str = "cache_hit";

#[derive(Debug, Error)]
pub enum CachedStreamEventsError {
    #[error("the cached stream event {index} is not UTF-8")]
    Utf8 {
        index: usize,
        #[source]
        source: Utf8Error,
    },
}

// LiteLLM's get_cached_stream_events: only a list or tuple carries events, and an absent one leaves the result unchanged
pub fn get_cached_stream_events(
    cached: &CachedDict,
) -> Result<Option<Vec<String>>, CachedStreamEventsError> {
    let Some(CacheValue::Array(items)) = cached.get(CACHED_STREAM_EVENTS_KEY) else {
        return Ok(None);
    };
    let mut events = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        match item {
            CacheValue::Text(text) => events.push(text.clone()),
            CacheValue::Bytes(bytes) => match str::from_utf8(bytes) {
                Ok(text) => events.push(text.to_owned()),
                Err(source) => return Err(CachedStreamEventsError::Utf8 { index, source }),
            },
            // LiteLLM keeps only text and bytes events, so any other cached value is skipped
            CacheValue::Credential(_)
            | CacheValue::Null
            | CacheValue::Boolean(_)
            | CacheValue::Integer(_)
            | CacheValue::Float(_)
            | CacheValue::Array(_)
            | CacheValue::Object(_) => {}
        }
    }
    Ok(Some(events))
}

// The logging side of a replay: LiteLLM's logging object, which __init__ stamps and __anext__ bills through
pub trait CachedStreamLogging: Send {
    fn stamp_cache_hit(&self);

    fn bill_cached_chunks(&self, collected_chunks: Vec<Vec<u8>>);
}

// LiteLLM's cached Anthropic messages iterator: the cached events replayed as the stream's chunks
pub struct CachedMessagesReplay<L> {
    chunks: Vec<Vec<u8>>,
    current_index: usize,
    logged: bool,
    logging: L,
}

impl<L: CachedStreamLogging> CachedMessagesReplay<L> {
    #[must_use]
    pub fn new(events: Vec<String>, logging: L) -> Self {
        Self {
            chunks: events.into_iter().map(String::into_bytes).collect(),
            current_index: 0,
            logged: false,
            logging,
        }
    }
}

#[async_trait]
impl<L> MessagesProviderStream for CachedMessagesReplay<L>
where
    L: CachedStreamLogging,
{
    type Error = Infallible;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Infallible> {
        if let Some(chunk) = self.chunks.get(self.current_index).cloned() {
            self.current_index = self.current_index.saturating_add(1);
            return Ok(Some(chunk));
        }
        if !self.logged {
            self.logged = true;
            self.logging.bill_cached_chunks(self.chunks.clone());
        }
        Ok(None)
    }
}

pub enum CachedMessagesResult<L> {
    Unchanged(CachedDict),
    Stream(CachedMessagesReplay<L>),
}

// LiteLLM's convert_cached_anthropic_messages_result; the request body already lives on the injected logging seam
pub fn convert_cached_messages_result<L>(
    cached: CachedDict,
    logging: L,
) -> Result<CachedMessagesResult<L>, CachedStreamEventsError>
where
    L: CachedStreamLogging,
{
    let Some(events) = get_cached_stream_events(&cached)? else {
        return Ok(CachedMessagesResult::Unchanged(cached));
    };
    logging.stamp_cache_hit();
    Ok(CachedMessagesResult::Stream(CachedMessagesReplay::new(
        events, logging,
    )))
}

// The production seam: B2436's stream logging bills the replayed chunks, and the logging object carries the cache_hit stamp
pub struct ReplayLogging<S> {
    pub logging_obj: Arc<StreamLoggingObject>,
    pub stream: MessagesStreamLogging<S>,
}

impl<S> CachedStreamLogging for ReplayLogging<S>
where
    S: StreamSeams,
    MessagesStreamLogging<S>: Send,
{
    fn stamp_cache_hit(&self) {
        self.logging_obj.edit(&mut |state| {
            state
                .model_call_details
                .insert(CACHE_HIT_KEY.to_owned(), Json::Bool(true));
        });
    }

    fn bill_cached_chunks(&self, collected_chunks: Vec<Vec<u8>>) {
        let _logged = self.stream.bill_collected_chunks(collected_chunks, false);
    }
}
