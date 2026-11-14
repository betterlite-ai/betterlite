use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::messages_native_stream::{
    MessagesChunkSource, StreamFailureCall, StreamFailureLogging,
};
use betterlite_core::api::messages_sse_events::stream::MessagesProviderStream;
use betterlite_core::api::messages_stream_logging::seams::{
    ChunkProcessorCall, LoggingFuture, ReportedFailure, RouteStreamingLogging, StreamSeams,
};
use betterlite_core::api::messages_stream_logging::slots::DetachedFailureHook;
use reqwest::Response;
use tokio::sync::watch;

use super::super::recording::Recorder;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct GateState {
    pub(super) permits: usize,
    pub(super) finished: bool,
    pub(super) failure: Option<ReportedFailure>,
}

// The probes drive both sides, so the pump only advances on a release, a finish or a failure
pub(super) struct Gate {
    pub(super) state: watch::Sender<GateState>,
    pub(super) closed: AtomicBool,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            state: watch::channel(GateState::default()).0,
            closed: AtomicBool::new(false),
        }
    }
}

pub(super) struct GateSource {
    pub(super) frames: Vec<Vec<u8>>,
    pub(super) index: usize,
    pub(super) gate: Arc<Gate>,
}

#[async_trait]
impl MessagesProviderStream for GateSource {
    type Error = ReportedFailure;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Self::Error> {
        let mut viewer = self.gate.state.subscribe();
        loop {
            let state = viewer.borrow_and_update().clone();
            if let Some(failure) = state.failure {
                return Err(failure);
            }
            if state.permits > 0 {
                self.gate
                    .state
                    .send_modify(|released| released.permits = released.permits.saturating_sub(1));
                let frame = self.frames.get(self.index).cloned();
                self.index = self.index.saturating_add(1);
                return Ok(frame);
            }
            if state.finished {
                return Ok(None);
            }
            if viewer.changed().await.is_err() {
                return Ok(None);
            }
        }
    }

    async fn aclose(&mut self) -> Option<Result<(), Self::Error>> {
        self.gate.closed.store(true, Ordering::SeqCst);
        Some(Ok(()))
    }
}

impl MessagesChunkSource for GateSource {}

// The probe reads the gate after the pump parked, which it can only do once it took every permit
pub(super) async fn quiesce(signal: &watch::Sender<GateState>) {
    let mut viewer = signal.subscribe();
    drop(
        viewer
            .wait_for(|state| state.finished || state.permits == 0)
            .await,
    );
}

// One recorder per stream, so each probe observes only its own billing calls
pub(super) struct StreamState {
    pub(super) recorder: Recorder,
    pub(super) failures: Mutex<Vec<StreamFailureCall>>,
    pub(super) hook_calls: Mutex<Vec<String>>,
    pub(super) recorded: watch::Sender<bool>,
    pub(super) fired: watch::Sender<bool>,
}

impl Default for StreamState {
    fn default() -> Self {
        Self {
            recorder: Recorder::default(),
            failures: Mutex::new(Vec::new()),
            hook_calls: Mutex::new(Vec::new()),
            recorded: watch::channel(false).0,
            fired: watch::channel(false).0,
        }
    }
}

impl StreamSeams for StreamState {
    type Response = Response;
    type Stream = Response;

    fn route_streaming_logging(
        &self,
        call: RouteStreamingLogging,
    ) -> Result<LoggingFuture, ReportedFailure> {
        let future = self.recorder.route_streaming_logging(call)?;
        self.recorded.send_modify(|recorded| *recorded = true);
        Ok(future)
    }

    fn chunk_processor(&self, call: ChunkProcessorCall<Response>) -> Response {
        self.recorder.chunk_processor(call)
    }
}

#[async_trait]
impl StreamFailureLogging for StreamState {
    async fn schedule_failure_logging(&self, call: StreamFailureCall) {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        self.recorded.send_modify(|recorded| *recorded = true);
    }
}

#[async_trait]
impl DetachedFailureHook for StreamState {
    async fn on_detached_failure(
        &self,
        exception: &ReportedFailure,
    ) -> Result<(), ReportedFailure> {
        self.hook_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(exception.to_string());
        self.fired.send_modify(|fired| *fired = true);
        Ok(())
    }
}
