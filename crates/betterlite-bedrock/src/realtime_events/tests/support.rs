use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::realtime_contracts::events::{
    RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use tracing::subscriber::{DefaultGuard, set_default};
use uuid::Uuid;

use crate::agentcore_stream::IdSource;
use crate::realtime_events::ack::{AckError, SessionAck};
use crate::realtime_events::client::{ClientError, RealtimeClientWebSocket};
use crate::realtime_events::decode::{decode_realtime_client_preface, json_dict, json_str};
use crate::realtime_events::dispatch::RealtimeEvents;
use crate::realtime_events::error::EventError;
use crate::realtime_events::scope::EventScope;
use crate::realtime_events::stream::{
    BedrockBidirectionalStream, BedrockInputStream, BedrockOutputChunk, BedrockOutputStream,
    ClosableBedrockRuntimeClient, StreamError,
};
use crate::realtime_session::RealtimeSession;

pub(super) const SECRET: &str = "sk-abcdefghijklmno";

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

pub(super) fn numbered(prefix: &str, number: u128) -> String {
    format!("{prefix}_{}", Uuid::from_u128(number))
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor")
}

pub(super) fn new_session() -> RealtimeSession {
    RealtimeSession::new(&CountingIds::new())
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

pub(super) fn input_with_ids(
    item: &str,
    response: &str,
    conversation: &str,
) -> RealtimeResponseTransformInput {
    RealtimeResponseTransformInput {
        current_output_item_id: Some(item.to_owned()),
        current_response_id: Some(response.to_owned()),
        current_conversation_id: Some(conversation.to_owned()),
        ..input()
    }
}

pub(super) fn transform(
    session: &mut RealtimeSession,
    ids: &CountingIds,
    message: &str,
    input: &RealtimeResponseTransformInput,
) -> Result<RealtimeResponseTypedDict, EventError> {
    let redactor = redactor();
    let mut events = RealtimeEvents {
        session,
        ids,
        redactor: &redactor,
    };
    events.transform_realtime_response(message.as_bytes(), input)
}

pub(super) fn transform_events(message: &str, input: &RealtimeResponseTransformInput) -> Vec<Json> {
    let ids = CountingIds::new();
    let mut session = new_session();
    let response = transform(&mut session, &ids, message, input).expect("a dispatched event");
    events(&response).to_vec()
}

pub(super) fn events(response: &RealtimeResponseTypedDict) -> &[Json] {
    match &response.response {
        Json::List(events) => events,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            panic!("the response is a list")
        }
    }
}

pub(super) fn event_type(event: &Json) -> Option<&str> {
    json_dict(event)
        .and_then(|fields| fields.get("type"))
        .and_then(|value| json_str(value))
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("valid JSON")
}

pub(super) fn preface(text: &str) -> IndexMap<String, Json> {
    decode_realtime_client_preface(text.as_bytes())
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).expect("captured log text")
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

pub(super) enum Received {
    Text(String),
    Error(String),
}

pub(super) struct FakeClient {
    pub(super) received: Mutex<VecDeque<Received>>,
    pub(super) sent: Mutex<Vec<String>>,
    pub(super) send_error: Option<String>,
}

impl FakeClient {
    pub(super) fn new() -> Self {
        Self {
            received: Mutex::new(VecDeque::new()),
            sent: Mutex::new(Vec::new()),
            send_error: None,
        }
    }

    pub(super) fn sent(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl RealtimeClientWebSocket for FakeClient {
    async fn receive_text(&self) -> Result<String, ClientError> {
        let mut received = self.received.lock().unwrap();
        match received.pop_front() {
            Some(Received::Text(text)) => Ok(text),
            Some(Received::Error(message)) => Err(ClientError { message }),
            None => Err(ClientError {
                message: "no more messages".to_owned(),
            }),
        }
    }

    async fn send_text(&self, data: String) -> Result<(), ClientError> {
        self.sent.lock().unwrap().push(data);
        match &self.send_error {
            Some(message) => Err(ClientError {
                message: message.clone(),
            }),
            None => Ok(()),
        }
    }

    async fn close(&self, _code: u16, _reason: Option<String>) {}
}

pub(super) struct FakeInput;

#[async_trait]
impl BedrockInputStream for FakeInput {
    async fn send(&self, _event: Json) -> Result<(), StreamError> {
        Ok(())
    }

    async fn close(&self) -> Result<(), StreamError> {
        Ok(())
    }
}

pub(super) struct FakeOutput;

#[async_trait]
impl BedrockOutputStream for FakeOutput {
    async fn receive(&self) -> Result<Option<Box<dyn BedrockOutputChunk>>, StreamError> {
        Ok(None)
    }
}

pub(super) struct FakeStream {
    pub(super) input: FakeInput,
    pub(super) error: Option<String>,
    pub(super) awaited: Mutex<u32>,
}

impl FakeStream {
    pub(super) fn new() -> Self {
        Self {
            input: FakeInput,
            error: None,
            awaited: Mutex::new(0),
        }
    }

    pub(super) fn awaited(&self) -> u32 {
        *self.awaited.lock().unwrap()
    }
}

#[async_trait]
impl BedrockBidirectionalStream for FakeStream {
    fn input_stream(&self) -> &dyn BedrockInputStream {
        &self.input
    }

    async fn await_output(&self) -> Result<(Json, Box<dyn BedrockOutputStream>), StreamError> {
        *self.awaited.lock().unwrap() += 1;
        match &self.error {
            Some(message) => Err(StreamError {
                message: message.clone(),
            }),
            None => Ok((Json::Null, Box::new(FakeOutput))),
        }
    }
}

pub(super) struct Closable {
    pub(super) error: Option<String>,
}

#[async_trait]
impl ClosableBedrockRuntimeClient for Closable {
    async fn close(&self) -> Result<(), StreamError> {
        match &self.error {
            Some(message) => Err(StreamError {
                message: message.clone(),
            }),
            None => Ok(()),
        }
    }
}

pub(super) struct AckSetup {
    pub(super) client: FakeClient,
    pub(super) stream: FakeStream,
    pub(super) scope: EventScope,
    pub(super) trace_id: Option<String>,
    pub(super) message: IndexMap<String, Json>,
    pub(super) ids: CountingIds,
}

impl AckSetup {
    pub(super) fn new() -> Self {
        Self {
            client: FakeClient::new(),
            stream: FakeStream::new(),
            scope: EventScope::default(),
            trace_id: None,
            message: IndexMap::new(),
            ids: CountingIds::new(),
        }
    }

    pub(super) async fn run(&mut self) -> Result<bool, AckError> {
        let redactor = redactor();
        let mut ack = SessionAck {
            client: &self.client,
            stream: &self.stream,
            scope: &mut self.scope,
            model: "amazon.nova-sonic-v1:0",
            trace_id: self.trace_id.as_deref(),
            message: &self.message,
            ids: &self.ids,
            redactor: &redactor,
        };
        ack.ack_session_update().await
    }
}
