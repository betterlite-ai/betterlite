use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::passthrough_stream_logging::build::StreamLoggingInput;
use crate::passthrough_stream_logging::router::StreamLoggingRouter;
use crate::worker::{Admission, Callback, LoggingWorker};

pub type StreamWork = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

// One logging coroutine on its way to B191's worker; the enqueue and the dispatch never share a call
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamJobKind {
    Success,
    Failure,
}

pub struct StreamLoggingJob {
    pub kind: StreamJobKind,
    pub work: StreamWork,
}

impl Callback for StreamLoggingJob {
    type Error = Infallible;

    fn name(&self) -> &str {
        match self.kind {
            StreamJobKind::Success => "passthrough_stream_success",
            StreamJobKind::Failure => "passthrough_stream_failure",
        }
    }

    async fn call(self) -> Result<(), Self::Error> {
        self.work.await;
        Ok(())
    }
}

// GLOBAL_LOGGING_WORKER.ensure_initialized_and_enqueue: the shared worker decides admission for every job
pub trait StreamLoggingQueue: Send + Sync {
    fn submit(&self, job: StreamLoggingJob) -> Admission;
}

impl StreamLoggingQueue for LoggingWorker<StreamLoggingJob> {
    fn submit(&self, job: StreamLoggingJob) -> Admission {
        Self::enqueue(self, job)
    }
}

// LiteLLM's _on_deferred_stream_complete: post-call guardrails arm it and the stream parks its logging there
pub trait DeferredStreamLogging: Send + Sync {
    fn park(&self, job: StreamLoggingJob);
}

// LiteLLM's route_streaming_logging parameter: the default is B3608's _route_streaming_logging_to_handler
pub trait StreamRoute: Send + Sync {
    fn route(self: Arc<Self>, input: StreamLoggingInput) -> StreamWork;
}

impl StreamRoute for StreamLoggingRouter {
    fn route(self: Arc<Self>, input: StreamLoggingInput) -> StreamWork {
        Box::pin(async move {
            self.route_streaming_logging_to_handler(input).await;
        })
    }
}
