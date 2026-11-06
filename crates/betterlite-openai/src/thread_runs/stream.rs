use std::pin::Pin;
use std::sync::Arc;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::StreamExt as _;
use futures_util::stream::Stream;

use crate::assistants::client::ClientKind;
use crate::thread_runs::decode::{FrameOutcome, RunFrames, decode_run_event};
use crate::thread_runs::error::ThreadRunsError;
use crate::thread_runs::records::RunEvent;

pub type RunChunks = Pin<Box<dyn Stream<Item = Result<Vec<u8>, ThreadRunsError>> + Send>>;
// The SDK manager yields each event as the provider writes it, so the stream seam hands back chunks.
pub trait RunStreamTransport: Send + Sync {
    fn open(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<RunChunks, ThreadRunsError>> + Send;
}
impl RunStreamTransport for ProxiedClient {
    async fn open(&self, request: OutboundRequest) -> Result<RunChunks, ThreadRunsError> {
        let response = self
            .send(request)
            .await
            .map_err(ThreadRunsError::Transport)?;
        let status = response.response.status().as_u16();
        if status >= 400 {
            let failure = response
                .buffered(self.limits.body_bytes)
                .await
                .map_err(ThreadRunsError::Transport)?;
            return Err(ThreadRunsError::Status(status, failure.body));
        }
        let chunks = response
            .body_stream()
            .map(|chunk| chunk.map_err(chunk_error));
        Ok(chunks.boxed())
    }
}
fn chunk_error(source: HttpError) -> ThreadRunsError {
    match source {
        HttpError::Timeout => ThreadRunsError::Timeout,
        HttpError::Request(error) if error.is_timeout() => ThreadRunsError::Timeout,
        rejected @ (HttpError::Required { .. }
        | HttpError::DotSegment { .. }
        | HttpError::UnsupportedProxyScheme { .. }
        | HttpError::InvalidProxyUrl { .. }
        | HttpError::Client(_)
        | HttpError::Request(_)
        | HttpError::TooLarge { .. }) => {
            ThreadRunsError::Transport(TransportError::Rejected(rejected))
        }
    }
}
// B161 bridges a Python event handler onto this seam; the stream reports every event here.
pub trait RunEventHandler: Send + Sync {
    fn on_event(&self, event: &RunEvent);
    fn on_exception(&self, error: &ThreadRunsError);
    fn on_timeout(&self);
    fn on_end(&self);
}
// The lazy manager the run wrappers return: nothing reaches the provider until it is consumed.
pub struct RunStream<T> {
    pub kind: ClientKind,
    pub handler: Option<Arc<dyn RunEventHandler>>,
    pub transport: Arc<T>,
    pub request: OutboundRequest,
}
impl<T: RunStreamTransport> RunStream<T> {
    // Entry errors raise before the SDK's iteration, so callbacks only follow a started stream.
    pub async fn consume(self) -> Result<Vec<RunEvent>, ThreadRunsError> {
        let Self {
            transport,
            request,
            handler,
            ..
        } = self;
        let chunks = match transport.open(request).await {
            Ok(chunks) => chunks,
            Err(error) => return Err(error),
        };
        let outcome = read_events(handler.as_ref(), chunks).await;
        if let (Some(sink), Err(error)) = (handler.as_ref(), &outcome) {
            // The SDK reports a timeout before the exception it wraps, then ends the handler once.
            if matches!(error, ThreadRunsError::Timeout) {
                sink.on_timeout();
            }
            sink.on_exception(error);
        }
        if let Some(sink) = handler.as_ref() {
            sink.on_end();
        }
        outcome
    }
}
fn deliver(handler: Option<&Arc<dyn RunEventHandler>>, event: &RunEvent) {
    if let Some(sink) = handler {
        sink.on_event(event);
    }
}
async fn read_events(
    handler: Option<&Arc<dyn RunEventHandler>>,
    mut chunks: RunChunks,
) -> Result<Vec<RunEvent>, ThreadRunsError> {
    let mut frames = RunFrames::default();
    let mut events = Vec::new();
    while let Some(chunk) = chunks.next().await {
        for frame in frames.push(&chunk?)? {
            match decode_run_event(&frame)? {
                FrameOutcome::Event(event) => {
                    deliver(handler, &event);
                    events.push(*event);
                }
                FrameOutcome::End => return Ok(events),
                FrameOutcome::Skip => {}
            }
        }
    }
    Ok(events)
}
