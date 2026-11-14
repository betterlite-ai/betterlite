use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use betterlite_http::error::HttpError;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use chrono::{DateTime, FixedOffset};
use futures_util::Stream;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use thiserror::Error;

use super::events::GenAiEventDecoder;
use super::logging::GenAiStreamLogging;
use super::metadata::GenAiStreamMetadata;

#[derive(Debug, Error)]
pub enum GenAiStreamError {
    #[error("the provider stream could not be read")]
    Transport {
        #[source]
        source: HttpError,
    },
    #[error(transparent)]
    Lines(#[from] AlephAlphaError),
    #[error("a streamed event is longer than {limit} bytes")]
    EventTooLong { limit: usize },
    #[error("Use AsyncGoogleGenAIGenerateContentStreamingIterator for async iteration")]
    AsyncIterationOnSyncStream,
    #[error("Use GoogleGenAIGenerateContentStreamingIterator for sync iteration")]
    SyncIterationOnAsyncStream,
}

pub struct GoogleGenAiStream {
    pub(crate) collected_chunks: Vec<Vec<u8>>,
    pub(crate) metadata: GenAiStreamMetadata,
    decoder: GenAiEventDecoder,
    source: Box<dyn Iterator<Item = Result<Vec<u8>, HttpError>> + Send>,
    failure: Option<GenAiStreamError>,
    finished: bool,
    async_pull_rejected: bool,
}

impl GoogleGenAiStream {
    // LiteLLM's GoogleGenAIGenerateContentStreamingIterator, fed the lines response.iter_lines() yields
    #[must_use]
    pub fn new<Source>(source: Source, metadata: GenAiStreamMetadata, line_limit: usize) -> Self
    where
        Source: Iterator<Item = Result<Vec<u8>, HttpError>> + Send + 'static,
    {
        Self {
            collected_chunks: Vec::new(),
            metadata,
            decoder: GenAiEventDecoder::new(line_limit),
            source: Box::new(source),
            failure: None,
            finished: false,
            async_pull_rejected: false,
        }
    }

    // the chunks the stream collected for the pass-through completion log
    #[must_use]
    pub fn collected_chunks(&self) -> &[Vec<u8>] {
        &self.collected_chunks
    }
}

impl Iterator for GoogleGenAiStream {
    type Item = Result<Vec<u8>, GenAiStreamError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.decoder.next_event() {
                Ok(Some(event)) => {
                    self.collected_chunks.push(event.clone());
                    return Some(Ok(event));
                }
                Ok(None) => {}
                Err(failure) => self.failure = Some(failure),
            }
            if let Some(failure) = self.failure.take() {
                self.finished = true;
                return Some(Err(failure));
            }
            if self.finished {
                return None;
            }
            match self.source.next() {
                Some(Ok(chunk)) => self.failure = self.decoder.push(&chunk).err(),
                Some(Err(source)) => {
                    self.finished = true;
                    return Some(Err(GenAiStreamError::Transport { source }));
                }
                None => {
                    self.decoder.finish();
                    self.finished = true;
                }
            }
        }
    }
}

impl Stream for GoogleGenAiStream {
    type Item = Result<Vec<u8>, GenAiStreamError>;

    // LiteLLM's sync iterator answers __anext__ with NotImplementedError before it touches the stream
    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.async_pull_rejected {
            return Poll::Ready(None);
        }
        self.async_pull_rejected = true;
        Poll::Ready(Some(Err(GenAiStreamError::AsyncIterationOnSyncStream)))
    }
}

pub struct AsyncGoogleGenAiStream {
    pub(crate) collected_chunks: Vec<Vec<u8>>,
    pub(crate) metadata: GenAiStreamMetadata,
    start_time: DateTime<FixedOffset>,
    decoder: GenAiEventDecoder,
    source: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    logging: Option<GenAiStreamLogging>,
    clock: Arc<dyn SuccessClock>,
    failure: Option<GenAiStreamError>,
    finished: bool,
    sync_pull_rejected: bool,
}

impl AsyncGoogleGenAiStream {
    // LiteLLM's AsyncGoogleGenAIGenerateContentStreamingIterator, fed the lines response.aiter_lines() yields
    #[must_use]
    pub fn new(
        source: BoxStream<'static, Result<Vec<u8>, HttpError>>,
        metadata: GenAiStreamMetadata,
        logging: Option<GenAiStreamLogging>,
        clock: Arc<dyn SuccessClock>,
        line_limit: usize,
    ) -> Self {
        Self {
            collected_chunks: Vec::new(),
            metadata,
            start_time: clock.now(),
            decoder: GenAiEventDecoder::new(line_limit),
            source,
            logging,
            clock,
            failure: None,
            finished: false,
            sync_pull_rejected: false,
        }
    }

    // the chunks the stream collected for the pass-through completion log
    #[must_use]
    pub fn collected_chunks(&self) -> &[Vec<u8>] {
        &self.collected_chunks
    }

    // LiteLLM's __anext__: one event per pull, then the collected bytes reach the pass-through logging handler
    pub async fn next_event(&mut self) -> Result<Option<Vec<u8>>, GenAiStreamError> {
        loop {
            match self.decoder.next_event() {
                Ok(Some(event)) => {
                    self.collected_chunks.push(event.clone());
                    return Ok(Some(event));
                }
                Ok(None) => {}
                Err(failure) => self.failure = Some(failure),
            }
            if let Some(failure) = self.failure.take() {
                self.finished = true;
                // an errored stream never reaches StopAsyncIteration, so LiteLLM logs nothing for it
                self.logging = None;
                return Err(failure);
            }
            if self.finished {
                self.log_at_eof();
                return Ok(None);
            }
            match self.source.next().await {
                Some(Ok(chunk)) => self.failure = self.decoder.push(&chunk).err(),
                Some(Err(source)) => {
                    self.finished = true;
                    self.logging = None;
                    return Err(GenAiStreamError::Transport { source });
                }
                None => {
                    self.decoder.finish();
                    self.finished = true;
                }
            }
        }
    }

    // LiteLLM's _handle_async_streaming_logging runs in a task of its own, so the end of the stream never waits on it
    fn log_at_eof(&mut self) {
        let Some(logging) = self.logging.take() else {
            return;
        };
        let end_time = self.clock.now();
        logging.schedule_route(
            self.metadata.clone(),
            self.collected_chunks.clone(),
            self.start_time,
            end_time,
        );
    }
}

impl Iterator for AsyncGoogleGenAiStream {
    type Item = Result<Vec<u8>, GenAiStreamError>;

    // a sync pull cannot await the provider, so it is rejected once and the stream then reports its end
    fn next(&mut self) -> Option<Self::Item> {
        if self.sync_pull_rejected {
            return None;
        }
        self.sync_pull_rejected = true;
        Some(Err(GenAiStreamError::SyncIterationOnAsyncStream))
    }
}

mod tests;
