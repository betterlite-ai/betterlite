use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::StreamExt as _;
use futures_util::stream::{Stream, unfold};
use tokio::time::timeout;

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
// The lazy manager the run wrappers return: nothing reaches the provider until it is opened.
pub struct RunStream<T> {
    pub kind: ClientKind,
    pub handler: Option<Arc<dyn RunEventHandler>>,
    pub transport: Arc<T>,
    pub request: OutboundRequest,
    pub read_timeout: Duration,
    pub max_bytes: usize,
}
impl<T: RunStreamTransport> RunStream<T> {
    // Entry errors raise before the SDK's iteration, so callbacks only follow a started stream.
    pub async fn events(self) -> Result<RunEventStream, ThreadRunsError> {
        let chunks = self.transport.open(self.request).await?;
        let open = OpenStream {
            chunks,
            frames: RunFrames::default(),
            pending: VecDeque::new(),
            handler: self.handler,
            read_timeout: self.read_timeout,
            max_bytes: self.max_bytes,
            total: 0,
        };
        Ok(Box::pin(unfold(Some(open), next_event)))
    }
    pub async fn consume(self) -> Result<Vec<RunEvent>, ThreadRunsError> {
        let mut events = self.events().await?;
        let mut collected = Vec::new();
        while let Some(event) = events.next().await {
            collected.push(event?);
        }
        Ok(collected)
    }
}
pub type RunEventStream = Pin<Box<dyn Stream<Item = Result<RunEvent, ThreadRunsError>> + Send>>;
pub(crate) const MAX_EVENT_BYTES: usize = 0x0100_0000;
struct OpenStream {
    chunks: RunChunks,
    frames: RunFrames,
    pending: VecDeque<Vec<u8>>,
    handler: Option<Arc<dyn RunEventHandler>>,
    read_timeout: Duration,
    max_bytes: usize,
    total: usize,
}
// The caller's timeout covers each read, so a live stream can run past it as long as chunks keep coming.
async fn pull(stream: &mut OpenStream) -> Result<Option<Vec<Vec<u8>>>, ThreadRunsError> {
    let next = timeout(stream.read_timeout, stream.chunks.next()).await;
    match next {
        Err(_elapsed) => Err(ThreadRunsError::Timeout),
        Ok(None) => Ok(None),
        Ok(Some(Err(error))) => Err(error),
        Ok(Some(Ok(chunk))) => {
            stream.total = stream.total.saturating_add(chunk.len());
            if stream.total > stream.max_bytes {
                return Err(ThreadRunsError::StreamTooLarge);
            }
            Ok(Some(stream.frames.push(&chunk)))
        }
    }
}
async fn next_event(
    state: Option<OpenStream>,
) -> Option<(Result<RunEvent, ThreadRunsError>, Option<OpenStream>)> {
    let mut stream = state?;
    loop {
        if let Some(frame) = stream.pending.pop_front() {
            match decode_run_event(&frame) {
                Ok(FrameOutcome::Skip) => continue,
                Ok(FrameOutcome::Event(event)) => {
                    deliver(stream.handler.as_ref(), &event);
                    return Some((Ok(*event), Some(stream)));
                }
                Ok(FrameOutcome::End) => {
                    finish(&stream);
                    return None;
                }
                Err(error) => {
                    fail(&stream, &error);
                    return Some((Err(error), None));
                }
            }
        }
        match pull(&mut stream).await {
            Ok(Some(frames)) => stream.pending.extend(frames),
            Ok(None) => {
                finish(&stream);
                return None;
            }
            Err(error) => {
                fail(&stream, &error);
                return Some((Err(error), None));
            }
        }
    }
}
// The SDK reports a timeout first, then the exception, and always ends the handler after a failure.
fn fail(stream: &OpenStream, error: &ThreadRunsError) {
    if let Some(sink) = stream.handler.as_ref() {
        if matches!(error, ThreadRunsError::Timeout) {
            sink.on_timeout();
        }
        sink.on_exception(error);
        sink.on_end();
    }
}
fn deliver(handler: Option<&Arc<dyn RunEventHandler>>, event: &RunEvent) {
    if let Some(sink) = handler {
        sink.on_event(event);
    }
}
fn finish(stream: &OpenStream) {
    if let Some(sink) = stream.handler.as_ref() {
        sink.on_end();
    }
}
