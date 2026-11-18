use std::collections::VecDeque;
use std::future::pending;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_bedrock::realtime_bridge::client::ClientForward;
use betterlite_bedrock::realtime_bridge::end::ForwardEnd;
use betterlite_bedrock::realtime_bridge::error::{ClientForwardError, ProviderForwardError};
use betterlite_bedrock::realtime_bridge::provider::{LoggedEventSink, ProviderForward};
use betterlite_bedrock::realtime_events::client::{ClientError, RealtimeClientWebSocket};
use betterlite_bedrock::realtime_events::scope::EventScope;
use betterlite_bedrock::realtime_events::stream::{
    BedrockBidirectionalStream, BedrockInputStream, BedrockOutputChunk, BedrockOutputStream,
    BedrockPayloadPart, StreamError,
};
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_contracts::events::RealtimeResponseTransformInput;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use tokio::runtime::{Builder, Runtime};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    BridgeView, Chunk, ChunkKind, ClosedClient, Input, Kind, Run, ScopeView, empty_state, failure,
    session_input,
};
use crate::services_realtime_session::FixedIds;
use betterlite_oracle_helper_api::error::HelperError;

struct ClientState {
    frames: Mutex<VecDeque<Option<String>>>,
    block: bool,
    client_send_failures: Vec<usize>,
    sent: Mutex<Vec<String>>,
    closes: Mutex<Vec<(u16, Option<String>)>>,
    receives: AtomicUsize,
}

// Records the value and answers the index when the case asked this send to fail
fn push_sent<T>(sent: &Mutex<Vec<T>>, failures: &[usize], value: T) -> Option<usize> {
    let mut sent = sent.lock().unwrap_or_else(PoisonError::into_inner);
    let index = sent.len();
    sent.push(value);
    failures.contains(&index).then_some(index)
}

impl ClientState {
    fn sent(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn closes(&self) -> Vec<(u16, Option<String>)> {
        self.closes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct TransportClient {
    state: Arc<ClientState>,
}

#[async_trait]
impl RealtimeClientWebSocket for TransportClient {
    type Error = io::Error;

    async fn receive_text(&self) -> Result<String, ClientError<io::Error>> {
        self.state.receives.fetch_add(1, Ordering::SeqCst);
        let frame = self
            .state
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        if let Some(Some(text)) = frame {
            return Ok(text);
        }
        if self.state.block {
            return pending().await;
        }
        Err(ClientError {
            message: "the client is gone".to_owned(),
            source: None,
        })
    }

    async fn send_text(&self, data: String) -> Result<(), ClientError<io::Error>> {
        let failed = push_sent(&self.state.sent, &self.state.client_send_failures, data);
        failed.map_or(Ok(()), |_index| {
            Err(ClientError {
                message: "the client send failed".to_owned(),
                source: None,
            })
        })
    }

    async fn close(&self, code: u16, reason: Option<String>) {
        self.state
            .closes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((code, reason));
    }
}

struct StreamState {
    sent: Mutex<Vec<Json>>,
    bedrock_send_failures: Vec<usize>,
    closed: AtomicBool,
    close_error: bool,
    await_error: Option<String>,
    chunks: Mutex<VecDeque<Chunk>>,
    receives: AtomicUsize,
    receive_failures: Vec<usize>,
    block: bool,
}

impl StreamState {
    fn sent(&self) -> Vec<Json> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct TransportInput {
    state: Arc<StreamState>,
}

#[async_trait]
impl BedrockInputStream for TransportInput {
    type Error = io::Error;

    async fn send(&self, event: Json) -> Result<(), StreamError<io::Error>> {
        if push_sent(&self.state.sent, &self.state.bedrock_send_failures, event).is_none() {
            return Ok(());
        }
        Err(StreamError {
            message: "the input send failed".to_owned(),
            source: None,
        })
    }

    async fn close(&self) -> Result<(), StreamError<io::Error>> {
        self.state.closed.store(true, Ordering::SeqCst);
        if self.state.close_error {
            return Err(StreamError {
                message: "the input close failed".to_owned(),
                source: None,
            });
        }
        Ok(())
    }
}

struct Part(Option<Vec<u8>>);

impl BedrockPayloadPart for Part {
    fn bytes(&self) -> Option<&[u8]> {
        self.0.as_deref()
    }
}

struct WireChunk(Option<Part>);

impl BedrockOutputChunk for WireChunk {
    fn value(&self) -> Option<&dyn BedrockPayloadPart> {
        let part: &dyn BedrockPayloadPart = self.0.as_ref()?;
        Some(part)
    }
}

struct TransportOutput {
    state: Arc<StreamState>,
}

#[async_trait]
impl BedrockOutputStream for TransportOutput {
    type Error = io::Error;

    async fn receive(&self) -> Result<Option<Box<dyn BedrockOutputChunk>>, StreamError<io::Error>> {
        let index = self.state.receives.fetch_add(1, Ordering::SeqCst);
        if self.state.receive_failures.contains(&index) {
            return Err(StreamError {
                message: "the provider receive failed".to_owned(),
                source: None,
            });
        }
        let chunk = self
            .state
            .chunks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match chunk {
            Some(chunk) => match chunk.kind {
                ChunkKind::End => Ok(None),
                ChunkKind::MissingValue => Ok(Some(Box::new(WireChunk(None)))),
                ChunkKind::MissingBytes => Ok(Some(Box::new(WireChunk(Some(Part(None)))))),
                ChunkKind::Empty => Ok(Some(Box::new(WireChunk(Some(Part(Some(Vec::new()))))))),
                ChunkKind::Bytes => Ok(Some(Box::new(WireChunk(Some(Part(Some(
                    chunk.text.into_bytes(),
                ))))))),
                ChunkKind::InvalidUtf8 => {
                    Ok(Some(Box::new(WireChunk(Some(Part(Some(vec![0xff])))))))
                }
            },
            None if self.state.block => pending().await,
            None => Ok(None),
        }
    }
}

struct TransportStream {
    state: Arc<StreamState>,
    input: TransportInput,
}

impl TransportStream {
    fn new(state: Arc<StreamState>) -> Self {
        Self {
            input: TransportInput {
                state: Arc::clone(&state),
            },
            state,
        }
    }
}

#[async_trait]
impl BedrockBidirectionalStream for TransportStream {
    type Error = io::Error;

    fn input_stream(&self) -> &dyn BedrockInputStream<Error = io::Error> {
        &self.input
    }

    async fn await_output(
        &self,
    ) -> Result<(Json, Box<dyn BedrockOutputStream<Error = io::Error>>), StreamError<io::Error>>
    {
        if let Some(message) = &self.state.await_error {
            return Err(StreamError {
                message: message.clone(),
                source: None,
            });
        }
        Ok((
            Json::Null,
            Box::new(TransportOutput {
                state: Arc::clone(&self.state),
            }),
        ))
    }
}

#[derive(Default)]
struct CollectedLog {
    events: Vec<Json>,
}

#[async_trait]
impl LoggedEventSink for CollectedLog {
    async fn record(&mut self, event: Json) {
        self.events.push(event);
    }
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)
}

fn threshold(run: &Run) -> usize {
    match run.kind {
        Kind::Client => run.frames.len().saturating_add(1),
        Kind::Provider => run.chunks.len().saturating_add(1),
    }
}

// Cancels once the scripted side has been pulled as many times as the run says
async fn cancel_at(counter: &AtomicUsize, threshold: usize, cancel: &CancellationToken) {
    while counter.load(Ordering::SeqCst) < threshold {
        tokio::task::yield_now().await;
    }
    cancel.cancel();
}

fn parse_sent(values: Vec<String>) -> Result<Vec<Json>, Box<HelperError>> {
    values
        .iter()
        .map(|text| serde_json::from_str(text).map_err(failure))
        .collect()
}

fn scope_view(scope: &EventScope) -> ScopeView {
    ScopeView {
        pending_session_update: match &scope.pending_session_update {
            Json::Text(text) => Some(text.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        },
        session_committed: matches!(scope.session_committed, Json::Bool(true)),
    }
}

enum PumpResult<'a> {
    Client(&'a Result<ForwardEnd, ClientForwardError<io::Error>>),
    Provider(&'a Result<ForwardEnd, ProviderForwardError<io::Error>>),
}

fn pump_outcome(result: PumpResult<'_>) -> (&'static str, Option<&'static str>) {
    match result {
        PumpResult::Client(result) => match result {
            Ok(ForwardEnd::Ended) => ("ended", None),
            Ok(ForwardEnd::Cancelled) => ("cancelled", None),
            Err(ClientForwardError::Input(_)) => ("error", Some("input")),
            Err(ClientForwardError::Ack(_)) => ("error", Some("ack")),
            Err(ClientForwardError::Send(_)) => ("error", Some("stream")),
            Err(ClientForwardError::Encode { .. }) => ("error", Some("encode")),
        },
        PumpResult::Provider(result) => match result {
            Ok(ForwardEnd::Ended) => ("ended", None),
            Ok(ForwardEnd::Cancelled) => ("cancelled", None),
            Err(ProviderForwardError::Stream(_)) => ("error", Some("stream")),
            Err(ProviderForwardError::Utf8 { .. }) => ("error", Some("utf8")),
            Err(ProviderForwardError::Transform(_)) => ("error", Some("transform")),
        },
    }
}

async fn view(
    run: &Run,
    name_ids: &[Uuid],
    events: &FixedIds,
    redactor: &Redactor,
    model: &str,
) -> Result<BridgeView, Box<HelperError>> {
    let shared = Mutex::new(session_input(name_ids));
    let client = Arc::new(ClientState {
        frames: Mutex::new(run.frames.iter().cloned().collect()),
        block: run.block,
        client_send_failures: run.client_send_failures.clone(),
        sent: Mutex::new(Vec::new()),
        closes: Mutex::new(Vec::new()),
        receives: AtomicUsize::new(0),
    });
    let wire = Arc::new(StreamState {
        sent: Mutex::new(Vec::new()),
        bedrock_send_failures: run.bedrock_send_failures.clone(),
        closed: AtomicBool::new(false),
        close_error: run.bedrock_close_error,
        await_error: run.await_error.clone(),
        chunks: Mutex::new(run.chunks.iter().cloned().collect()),
        receives: AtomicUsize::new(0),
        receive_failures: run.receive_failures.clone(),
        block: run.block,
    });
    let stream = TransportStream::new(Arc::clone(&wire));
    let transport = TransportClient {
        state: Arc::clone(&client),
    };
    let token = CancellationToken::new();
    match run.kind {
        Kind::Client => {
            let mut scope = EventScope::default();
            let mut forward: ClientForward<'_, io::Error, io::Error> = ClientForward {
                client: &transport,
                stream: &stream,
                shared: &shared,
                scope: &mut scope,
                model,
                trace_id: run.trace_id.as_deref(),
                ids: events,
                redactor,
                initial: run.initial.clone(),
                cancel: &token,
            };
            let result = if run.cancel {
                let canceller = cancel_at(&client.receives, threshold(run), &token);
                let pump = forward.forward_client_to_bedrock();
                tokio::pin!(pump);
                tokio::pin!(canceller);
                tokio::select! {
                    result = &mut pump => result,
                    () = &mut canceller => pump.await,
                }
            } else {
                forward.forward_client_to_bedrock().await
            };
            let (outcome, error) = pump_outcome(PumpResult::Client(&result));
            Ok(BridgeView {
                outcome,
                error,
                sent_to_bedrock: wire.sent(),
                sent_to_client: parse_sent(client.sent())?,
                closed_input: wire.closed.load(Ordering::SeqCst),
                closed_client: None,
                scope: Some(scope_view(&scope)),
                state: None,
                logged: Vec::new(),
            })
        }
        Kind::Provider => {
            let mut state: RealtimeResponseTransformInput = empty_state();
            let mut log = CollectedLog::default();
            let types = LoggedEventTypes::Default;
            let mut forward: ProviderForward<'_, io::Error, io::Error> = ProviderForward {
                stream: &stream,
                client: &transport,
                shared: &shared,
                state: &mut state,
                logged: &mut log,
                logged_types: &types,
                ids: events,
                redactor,
                cancel: &token,
            };
            let result = if run.cancel {
                let canceller = cancel_at(&wire.receives, threshold(run), &token);
                let pump = forward.forward_bedrock_to_client();
                tokio::pin!(pump);
                tokio::pin!(canceller);
                tokio::select! {
                    result = &mut pump => result,
                    () = &mut canceller => pump.await,
                }
            } else {
                forward.forward_bedrock_to_client().await
            };
            let (outcome, error) = pump_outcome(PumpResult::Provider(&result));
            let closed = client.closes();
            Ok(BridgeView {
                outcome,
                error,
                sent_to_bedrock: Vec::new(),
                sent_to_client: parse_sent(client.sent())?,
                closed_input: false,
                closed_client: closed.first().map(|(code, reason)| ClosedClient {
                    code: *code,
                    reason: reason.clone(),
                }),
                scope: None,
                state: Some(state),
                logged: log.events,
            })
        }
    }
}

pub(super) fn runs(
    input: &Input,
    names: &[Uuid],
    events: &FixedIds,
    redactor: &Redactor,
) -> Result<serde_json::Value, Box<HelperError>> {
    let executor = runtime()?;
    let views = executor.block_on(async {
        let mut views = Vec::with_capacity(input.runs.len());
        for (index, run) in input.runs.iter().enumerate() {
            let start = index.saturating_mul(3);
            let name_ids = names
                .get(start..start.saturating_add(3))
                .ok_or_else(|| failure(format!("run {index} needs three name ids")))?;
            views.push(view(run, name_ids, events, redactor, &input.model).await?);
        }
        Ok::<Vec<BridgeView>, Box<HelperError>>(views)
    })?;
    serde_json::to_value(super::Runs { runs: views }).map_err(failure)
}
