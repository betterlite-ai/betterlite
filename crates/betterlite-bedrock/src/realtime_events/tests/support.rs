use super::transport::{FakeClient, FakeStream};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

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
use crate::realtime_events::decode::{decode_realtime_client_preface, json_dict, json_str};
use crate::realtime_events::dispatch::RealtimeEvents;
use crate::realtime_events::error::EventError;
use crate::realtime_events::scope::EventScope;
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

// The bridge feeds each response's seven state fields back as the next dispatch input
pub(super) fn next_input(response: &RealtimeResponseTypedDict) -> RealtimeResponseTransformInput {
    RealtimeResponseTransformInput {
        session_configuration_request: response.session_configuration_request.clone(),
        current_output_item_id: response.current_output_item_id.clone(),
        current_response_id: response.current_response_id.clone(),
        current_delta_chunks: response.current_delta_chunks.clone(),
        current_item_chunks: response.current_item_chunks.clone(),
        current_conversation_id: response.current_conversation_id.clone(),
        current_delta_type: response.current_delta_type,
    }
}

pub(super) fn events(response: &RealtimeResponseTypedDict) -> &[Json] {
    let Json::List(events) = &response.response else {
        panic!("the response is a list");
    };
    events
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

    pub(super) async fn run(&mut self) -> Result<bool, AckError<io::Error>> {
        let redactor = redactor();
        let mut ack: SessionAck<'_, io::Error, io::Error> = SessionAck {
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
