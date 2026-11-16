use std::sync::Arc;

use betterlite_http::error::HttpError;
use http::HeaderMap;

use super::flush_scheduling::start_sync_flush;
use super::logging::{SpendLogging, SyncFlushExecutor};
use super::response::{SyncChunkSource, SyncResponse};
use super::spend_collection::{SpendCollection, SpendSink};

mod tests;

pub struct PassthroughStreamingResponse<S, L, E>
where
    S: SpendSink,
    L: SpendLogging<S::Logged>,
{
    pub status_code: u16,
    pub headers: HeaderMap,
    pub source: Box<dyn SyncChunkSource>,
    pub spend: SpendCollection<S>,
    pub logging: Arc<L>,
    pub executor: E,
    pub flush_scheduled: bool,
}

impl<S, L, E> PassthroughStreamingResponse<S, L, E>
where
    S: SpendSink,
    L: SpendLogging<S::Logged>,
    E: SyncFlushExecutor,
{
    #[must_use]
    pub fn new(
        response: SyncResponse,
        spend: SpendCollection<S>,
        logging: Arc<L>,
        executor: E,
    ) -> Self {
        Self {
            status_code: response.status_code,
            headers: response.headers,
            source: response.source,
            spend,
            logging,
            executor,
            flush_scheduled: false,
        }
    }

    // LiteLLM's PassthroughStreamingResponse.__next__: status and headers are already in hand, bytes pass through
    pub fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, HttpError> {
        match self.source.next_chunk() {
            Ok(Some(chunk)) => {
                self.spend.add(&chunk);
                Ok(Some(chunk))
            }
            other => {
                start_sync_flush(self);
                self.close_response();
                other
            }
        }
    }

    // LiteLLM's close: schedule the flush once and close, so a finally plus a __del__ close is safe
    pub fn close(&mut self) {
        start_sync_flush(self);
        self.close_response();
    }

    fn close_response(&mut self) {
        if let Err(error) = self.source.close() {
            // Safe catch-all for cleanup logic: a close failure after the stream ended has nowhere to go
            tracing::debug!(%error, "the passthrough response could not be closed");
        }
    }
}

impl<S, L, E> Iterator for PassthroughStreamingResponse<S, L, E>
where
    S: SpendSink,
    L: SpendLogging<S::Logged>,
    E: SyncFlushExecutor,
{
    type Item = Result<Vec<u8>, HttpError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_chunk() {
            Ok(Some(chunk)) => Some(Ok(chunk)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }
}
