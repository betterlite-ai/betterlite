use std::sync::Arc;

use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;

use super::error::{PassthroughStreamError, UpstreamStatus};
use super::flush_scheduling::start_async_flush;
use super::logging::SpendLogging;
use super::response::{AsyncChunkSource, PendingRelay};
use super::spend_collection::{SpendCollection, SpendSink};

mod close;
mod init;
mod iteration;
mod properties;

pub struct AsyncPassthroughStream<S, L>
where
    S: SpendSink,
    L: SpendLogging<S::Logged>,
{
    pub initialized: bool,
    pub status_code: u16,
    pub headers: HeaderMap,
    pub pending: Option<PendingRelay>,
    pub source: Option<Box<dyn AsyncChunkSource>>,
    pub spend: SpendCollection<S>,
    pub logging: Arc<L>,
    // mutable-ok: instance set for background task tracking
    pub background_tasks: TaskTracker,
    pub flush_scheduled: bool,
    // mutable-ok: router attaches response headers here in place
    pub hidden_params: IndexMap<String, Json>,
}

impl<S, L> AsyncPassthroughStream<S, L>
where
    S: SpendSink,
    L: SpendLogging<S::Logged>,
{
    #[must_use]
    pub fn new(
        pending: PendingRelay,
        spend: SpendCollection<S>,
        logging: Arc<L>,
        background_tasks: TaskTracker,
    ) -> Self {
        Self {
            initialized: false,
            status_code: 0,
            headers: HeaderMap::new(),
            pending: Some(pending),
            source: None,
            spend,
            logging,
            background_tasks,
            flush_scheduled: false,
            hidden_params: IndexMap::new(),
        }
    }

    pub const fn status_code(&self) -> Result<u16, PassthroughStreamError> {
        if self.initialized {
            Ok(self.status_code)
        } else {
            Err(PassthroughStreamError::NotInitialized {
                field: "status_code",
            })
        }
    }

    pub const fn headers(&self) -> Result<&HeaderMap, PassthroughStreamError> {
        if self.initialized {
            Ok(&self.headers)
        } else {
            Err(PassthroughStreamError::NotInitialized { field: "headers" })
        }
    }

    // LiteLLM's __await__ / _init: read the response, expose it, then raise on a status a caller can't stream
    pub async fn init(&mut self) -> Result<(), PassthroughStreamError> {
        if self.initialized {
            return Ok(());
        }
        let Some(pending) = self.pending.take() else {
            return Err(PassthroughStreamError::AlreadyAwaited);
        };
        let response = pending.await?;
        self.headers = response.headers;
        self.status_code = response.status_code;
        self.initialized = true;
        let mut source = response.source;
        if (200..300).contains(&self.status_code) {
            self.source = Some(source);
            return Ok(());
        }
        let body = match source.read_body().await {
            Ok(body) => body,
            Err(error) => {
                tracing::debug!(%error, "the upstream error body could not be read");
                Vec::new()
            }
        };
        if let Err(error) = source.close().await {
            tracing::debug!(%error, "the upstream error response could not be closed");
        }
        Err(PassthroughStreamError::UpstreamStatus(Box::new(
            UpstreamStatus {
                status_code: self.status_code,
                headers: self.headers.clone(),
                body,
            },
        )))
    }

    // LiteLLM's __anext__: bytes pass through untouched while the spend collector sees a copy
    pub async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, PassthroughStreamError> {
        if !self.initialized {
            self.init().await?;
        }
        let Some(source) = self.source.as_mut() else {
            return Err(PassthroughStreamError::AlreadyAwaited);
        };
        let next = source.next_chunk().await;
        match next {
            Ok(Some(chunk)) => {
                self.spend.add(&chunk);
                Ok(Some(chunk))
            }
            other => {
                start_async_flush(self);
                self.close_response().await;
                other.map_err(|error| PassthroughStreamError::StreamRead(Box::new(error)))
            }
        }
    }

    // LiteLLM's aclose: schedule the flush once, then close; a cleanup error never masks the caller's own
    pub async fn aclose(&mut self) {
        start_async_flush(self);
        if !self.initialized {
            return;
        }
        self.close_response().await;
    }

    async fn close_response(&mut self) {
        let Some(source) = self.source.as_mut() else {
            return;
        };
        if let Err(error) = source.close().await {
            tracing::debug!(
                %error,
                "the passthrough response could not be closed after the stream ended"
            );
        }
    }
}
