use std::collections::VecDeque;
use std::future::{Future, pending};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_contracts::events::RealtimeResponseTransformInput;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::future::{Either, select};
use tokio::task::yield_now;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::agentcore_stream::IdSource;
use crate::realtime_bridge::client::ClientForward;
use crate::realtime_bridge::end::ForwardEnd;
use crate::realtime_bridge::error::{ClientForwardError, ProviderForwardError};
use crate::realtime_bridge::provider::{LoggedEventSink, ProviderForward};
use crate::realtime_events::client::{ClientError, RealtimeClientWebSocket};
use crate::realtime_events::scope::EventScope;
use crate::realtime_events::stream::{
    BedrockBidirectionalStream, BedrockInputStream, BedrockOutputChunk, BedrockOutputStream,
    BedrockPayloadPart, StreamError,
};
use crate::realtime_input::RealtimeInput;
use crate::realtime_session::RealtimeSession;

pub(super) const MODEL: &str = "amazon.nova-sonic-v1:0";

pub(super) struct CountingIds {
    next: Mutex<u128>,
}

impl CountingIds {
    pub(super) const fn new() -> Self {
        Self {
            next: Mutex::new(1),
        }
    }
}

impl IdSource for CountingIds {
    fn next_id(&self) -> Uuid {
        let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        let id = Uuid::from_u128(*next);
        *next = next.saturating_add(1);
        id
    }
}

pub(super) fn numbered(number: u128) -> String {
    Uuid::from_u128(number).to_string()
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor")
}

pub(super) fn input() -> RealtimeResponseTransformInput {
    RealtimeResponseTransformInput {
        session_configuration_request: None,
        current_output_item_id: None,
        current_response_id: None,
        current_delta_chunks: None,
        current_item_chunks: None,
        current_conversation_id: None,
        current_delta_type: None,
    }
}

pub(super) fn logged_types() -> LoggedEventTypes {
    LoggedEventTypes::Default
}

pub(super) type ClientRun = Result<ForwardEnd, ClientForwardError<io::Error>>;
pub(super) type ProviderRun = Result<ForwardEnd, ProviderForwardError<io::Error>>;

fn session(ids: &CountingIds) -> Mutex<RealtimeInput> {
    Mutex::new(RealtimeInput {
        session: RealtimeSession::new(ids),
        open_audio: None,
    })
}

#[derive(Default)]
pub(super) struct RecordedLog {
    pub events: Vec<Json>,
}

#[async_trait]
impl LoggedEventSink for RecordedLog {
    async fn record(&mut self, event: Json) {
        self.events.push(event);
    }
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("valid JSON")
}

pub(super) fn event_key(value: &Json) -> Option<&str> {
    let Json::Object(fields) = value else {
        return None;
    };
    let Json::Object(event) = fields.get("event")? else {
        return None;
    };
    event.keys().next().map(String::as_str)
}

struct ClientState {
    frames: Mutex<VecDeque<Option<String>>>,
    block_receive: bool,
    send_failure: Option<usize>,
    sends: AtomicUsize,
    sent: Mutex<Vec<String>>,
    closes: Mutex<Vec<(u16, Option<String>)>>,
}

pub(super) struct ScriptedClient {
    state: Arc<ClientState>,
    receives: Arc<AtomicUsize>,
}

impl ScriptedClient {
    pub(super) fn new(frames: Vec<Option<String>>) -> Self {
        Self::with_receive(frames, false)
    }

    pub(super) fn blocking() -> Self {
        Self::with_receive(Vec::new(), true)
    }

    fn with_receive(frames: Vec<Option<String>>, block_receive: bool) -> Self {
        Self {
            state: Arc::new(ClientState {
                frames: Mutex::new(frames.into_iter().collect()),
                block_receive,
                send_failure: None,
                sends: AtomicUsize::new(0),
                sent: Mutex::new(Vec::new()),
                closes: Mutex::new(Vec::new()),
            }),
            receives: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(super) fn failing_send(mut self, index: usize) -> Self {
        Arc::get_mut(&mut self.state)
            .expect("the only client state reference")
            .send_failure = Some(index);
        self
    }

    pub(super) fn sends(&self) -> Vec<String> {
        self.state
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn sent_events(&self) -> Vec<Json> {
        self.sends()
            .iter()
            .map(|text| serde_json::from_str(text).expect("a JSON client event"))
            .collect()
    }

    pub(super) fn closes(&self) -> Vec<(u16, Option<String>)> {
        self.state
            .closes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl RealtimeClientWebSocket for ScriptedClient {
    type Error = io::Error;

    async fn receive_text(&self) -> Result<String, ClientError<io::Error>> {
        self.receives.fetch_add(1, Ordering::SeqCst);
        let frame = self
            .state
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match frame {
            Some(Some(text)) => Ok(text),
            Some(None) | None if !self.state.block_receive => Err(ClientError {
                message: "the client is gone".to_owned(),
                source: None,
            }),
            Some(None) | None => pending().await,
        }
    }

    async fn send_text(&self, data: String) -> Result<(), ClientError<io::Error>> {
        let index = self.state.sends.fetch_add(1, Ordering::SeqCst);
        self.state
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(data);
        if self.state.send_failure == Some(index) {
            return Err(ClientError {
                message: "the client send failed".to_owned(),
                source: None,
            });
        }
        Ok(())
    }

    async fn close(&self, code: u16, reason: Option<String>) {
        self.state
            .closes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((code, reason));
    }
}

struct ScriptedInput {
    sent: Mutex<Vec<Json>>,
    send_failures: Vec<usize>,
    close_failure: bool,
    closed: AtomicBool,
}

#[async_trait]
impl BedrockInputStream for ScriptedInput {
    type Error = io::Error;

    async fn send(&self, event: Json) -> Result<(), StreamError<io::Error>> {
        let index = self
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
        if self.send_failures.contains(&index) {
            return Err(StreamError {
                message: "the input send failed".to_owned(),
                source: None,
            });
        }
        Ok(())
    }

    async fn close(&self) -> Result<(), StreamError<io::Error>> {
        self.closed.store(true, Ordering::SeqCst);
        if self.close_failure {
            return Err(StreamError {
                message: "the input close failed".to_owned(),
                source: None,
            });
        }
        Ok(())
    }
}

pub(super) enum Chunk {
    MissingValue,
    MissingBytes,
    Bytes(Vec<u8>),
    End,
}

impl Chunk {
    pub(super) fn bytes(text: &str) -> Self {
        Self::Bytes(text.as_bytes().to_vec())
    }
}

struct ScriptedPart(Option<Vec<u8>>);

impl BedrockPayloadPart for ScriptedPart {
    fn bytes(&self) -> Option<&[u8]> {
        self.0.as_deref()
    }
}

struct ScriptedChunk {
    part: Option<ScriptedPart>,
}

impl BedrockOutputChunk for ScriptedChunk {
    fn value(&self) -> Option<&dyn BedrockPayloadPart> {
        let part: &dyn BedrockPayloadPart = self.part.as_ref()?;
        Some(part)
    }
}

pub(super) struct OutputState {
    chunks: Mutex<VecDeque<Chunk>>,
    receives: AtomicUsize,
    receive_failure: Option<usize>,
    block_receive: bool,
}

pub(super) struct ScriptedOutput(Arc<OutputState>);

#[async_trait]
impl BedrockOutputStream for ScriptedOutput {
    type Error = io::Error;

    async fn receive(&self) -> Result<Option<Box<dyn BedrockOutputChunk>>, StreamError<io::Error>> {
        let index = self.0.receives.fetch_add(1, Ordering::SeqCst);
        if self.0.receive_failure == Some(index) {
            return Err(StreamError {
                message: "the provider receive failed".to_owned(),
                source: None,
            });
        }
        let chunk = self
            .0
            .chunks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match chunk {
            Some(Chunk::MissingValue) => Ok(Some(Box::new(ScriptedChunk { part: None }))),
            Some(Chunk::MissingBytes) => Ok(Some(Box::new(ScriptedChunk {
                part: Some(ScriptedPart(None)),
            }))),
            Some(Chunk::Bytes(bytes)) => Ok(Some(Box::new(ScriptedChunk {
                part: Some(ScriptedPart(Some(bytes))),
            }))),
            None if self.0.block_receive => pending().await,
            Some(Chunk::End) | None => Ok(None),
        }
    }
}

pub(super) struct ScriptedStream {
    input: ScriptedInput,
    output: Arc<OutputState>,
    await_failure: Option<String>,
}

impl ScriptedStream {
    pub(super) fn new(chunks: Vec<Chunk>) -> Self {
        Self {
            input: ScriptedInput {
                sent: Mutex::new(Vec::new()),
                send_failures: Vec::new(),
                close_failure: false,
                closed: AtomicBool::new(false),
            },
            output: Arc::new(OutputState {
                chunks: Mutex::new(chunks.into_iter().collect()),
                receives: AtomicUsize::new(0),
                receive_failure: None,
                block_receive: false,
            }),
            await_failure: None,
        }
    }

    pub(super) fn with_send_failures(mut self, indexes: &[usize]) -> Self {
        self.input.send_failures = indexes.to_vec();
        self
    }

    pub(super) fn with_close_failure(mut self) -> Self {
        self.input.close_failure = true;
        self
    }

    pub(super) fn with_await_failure(mut self, message: &str) -> Self {
        self.await_failure = Some(message.to_owned());
        self
    }

    pub(super) fn with_receive_failure(mut self, index: usize) -> Self {
        Arc::get_mut(&mut self.output)
            .expect("the only output reference")
            .receive_failure = Some(index);
        self
    }

    pub(super) fn blocking(mut self) -> Self {
        Arc::get_mut(&mut self.output)
            .expect("the only output reference")
            .block_receive = true;
        self
    }

    pub(super) fn sent(&self) -> Vec<Json> {
        self.input
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn closed(&self) -> bool {
        self.input.closed.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl BedrockBidirectionalStream for ScriptedStream {
    type Error = io::Error;

    fn input_stream(&self) -> &dyn BedrockInputStream<Error = io::Error> {
        &self.input
    }

    async fn await_output(
        &self,
    ) -> Result<(Json, Box<dyn BedrockOutputStream<Error = io::Error>>), StreamError<io::Error>>
    {
        if let Some(message) = &self.await_failure {
            return Err(StreamError {
                message: message.clone(),
                source: None,
            });
        }
        Ok((
            Json::Null,
            Box::new(ScriptedOutput(Arc::clone(&self.output))),
        ))
    }
}

pub(super) struct ClientFixture {
    pub ids: CountingIds,
    pub shared: Mutex<RealtimeInput>,
    pub scope: EventScope,
    pub redactor: Redactor,
    pub token: CancellationToken,
    pub client: ScriptedClient,
    pub stream: ScriptedStream,
    pub initial: Option<String>,
    pub trace_id: Option<String>,
}

impl ClientFixture {
    pub(super) fn new(initial: Option<String>, client: ScriptedClient) -> Self {
        let ids = CountingIds::new();
        let shared = session(&ids);
        Self {
            ids,
            shared,
            scope: EventScope::default(),
            redactor: redactor(),
            token: CancellationToken::new(),
            client,
            stream: ScriptedStream::new(Vec::new()),
            initial,
            trace_id: Some("trace-bridge-1".to_owned()),
        }
    }

    pub(super) async fn run(&mut self) -> ClientRun {
        let mut pump = self.pump();
        pump.forward_client_to_bedrock().await
    }

    fn pump(&mut self) -> ClientForward<'_, io::Error, io::Error> {
        ClientForward {
            client: &self.client,
            stream: &self.stream,
            shared: &self.shared,
            scope: &mut self.scope,
            model: MODEL,
            trace_id: self.trace_id.as_deref(),
            ids: &self.ids,
            redactor: &self.redactor,
            initial: self.initial.take(),
            cancel: &self.token,
        }
    }

    pub(super) async fn run_cancelled(&mut self, threshold: usize) -> ClientRun {
        let counter = Arc::clone(&self.client.receives);
        let token = self.token.clone();
        cancel_when(self.run(), cancel_at(&counter, threshold, &token)).await
    }
}

pub(super) struct ProviderFixture {
    pub ids: CountingIds,
    pub shared: Mutex<RealtimeInput>,
    pub state: RealtimeResponseTransformInput,
    pub log: RecordedLog,
    pub types: LoggedEventTypes,
    pub redactor: Redactor,
    pub token: CancellationToken,
    pub client: ScriptedClient,
    pub stream: ScriptedStream,
}

impl ProviderFixture {
    pub(super) fn new(client: ScriptedClient, stream: ScriptedStream) -> Self {
        let ids = CountingIds::new();
        let shared = session(&ids);
        Self {
            ids,
            shared,
            state: input(),
            log: RecordedLog::default(),
            types: logged_types(),
            redactor: redactor(),
            token: CancellationToken::new(),
            client,
            stream,
        }
    }

    pub(super) async fn run(&mut self) -> ProviderRun {
        let mut pump: ProviderForward<'_, io::Error, io::Error> = ProviderForward {
            stream: &self.stream,
            client: &self.client,
            shared: &self.shared,
            state: &mut self.state,
            logged: &mut self.log,
            logged_types: &self.types,
            ids: &self.ids,
            redactor: &self.redactor,
            cancel: &self.token,
        };
        pump.forward_bedrock_to_client().await
    }

    pub(super) async fn run_cancelled(&mut self, threshold: usize) -> ProviderRun {
        let output = Arc::clone(&self.stream.output);
        let token = self.token.clone();
        cancel_when(self.run(), cancel_at(&output.receives, threshold, &token)).await
    }
}

// Cancels once the scripted side has been pulled as many times as the case says
async fn cancel_at(counter: &AtomicUsize, threshold: usize, cancel: &CancellationToken) {
    while counter.load(Ordering::SeqCst) < threshold {
        yield_now().await;
    }
    cancel.cancel();
}

// The pump finishes on its own, or the watchdog cancels it and the pump sees the token and returns
async fn cancel_when<T, P, W>(pump: P, watching: W) -> T
where
    P: Future<Output = T>,
    W: Future<Output = ()>,
{
    tokio::pin!(pump);
    tokio::pin!(watching);
    match select(pump, watching).await {
        Either::Left((result, _watching)) => result,
        Either::Right(((), pinned)) => pinned.await,
    }
}
