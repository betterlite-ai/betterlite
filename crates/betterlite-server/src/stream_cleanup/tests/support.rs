use std::io;
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;
use tokio_util::task::TaskTracker;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;

use super::super::cleanup::DisconnectCleanup;
use super::super::ports::{CleanupError, CleanupFuture, RequestLogging, SlotRelease};
use crate::sse_data_generator::types::{CleanupOutcome, CollectedStream, StreamSnapshot};

pub(super) fn object(entries: &[(&str, Json)]) -> JsonObject {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(super) fn expected_error_info() -> Json {
    Json::Object(object(&[
        ("error_code", text("499")),
        ("error_message", text("Client disconnected the request")),
        ("error_class", text("ClientDisconnected")),
    ]))
}

pub(super) fn backfilled_usage() -> Json {
    Json::Object(object(&[
        ("prompt_tokens", number(10)),
        ("completion_tokens", number(5)),
        ("total_tokens", number(15)),
        ("completion_tokens_details", Json::Null),
        (
            "prompt_tokens_details",
            Json::Object(object(&[
                ("audio_tokens", Json::Null),
                ("cached_tokens", number(4)),
                ("text_tokens", Json::Null),
                ("image_tokens", Json::Null),
                ("video_tokens", Json::Null),
            ])),
        ),
        ("cache_read_input_tokens", number(4)),
        ("cache_creation_input_tokens", number(0)),
    ]))
}

fn metadata(container: &JsonObject) -> Option<&JsonObject> {
    match container.get("metadata") {
        Some(Json::Object(metadata)) => Some(metadata),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

fn disconnect_marked(request_data: &JsonObject) -> bool {
    metadata(request_data)
        .and_then(|metadata| metadata.get("client_disconnected"))
        .is_some_and(|value| *value == Json::Bool(true))
}

#[derive(Default)]
struct LogState {
    details: Option<JsonObject>,
    armed: bool,
    baseline_active: bool,
    assemble: Option<Result<Option<JsonObject>, CleanupError>>,
    dispatch: Option<Result<(), CleanupError>>,
    invalidate: Option<Result<(), CleanupError>>,
    events: Vec<String>,
    fired: usize,
    fired_with: Option<JsonObject>,
    dispatched: Vec<JsonObject>,
    assembled: Option<(usize, Option<Vec<Json>>, bool)>,
}

pub(super) struct FakeLogging {
    state: Mutex<LogState>,
}

impl FakeLogging {
    pub(super) fn new(details: Option<JsonObject>) -> Self {
        Self {
            state: Mutex::new(LogState {
                details,
                ..LogState::default()
            }),
        }
    }

    pub(super) fn with_armed(self, armed: bool) -> Self {
        self.state.lock().unwrap().armed = armed;
        self
    }

    pub(super) fn with_baseline(self, active: bool) -> Self {
        self.state.lock().unwrap().baseline_active = active;
        self
    }

    pub(super) fn with_assemble(self, result: Result<Option<JsonObject>, CleanupError>) -> Self {
        self.state.lock().unwrap().assemble = Some(result);
        self
    }

    pub(super) fn with_dispatch(self, result: Result<(), CleanupError>) -> Self {
        self.state.lock().unwrap().dispatch = Some(result);
        self
    }

    pub(super) fn with_invalidate(self, result: Result<(), CleanupError>) -> Self {
        self.state.lock().unwrap().invalidate = Some(result);
        self
    }

    pub(super) fn events(&self) -> Vec<String> {
        self.state.lock().unwrap().events.clone()
    }

    pub(super) fn details(&self) -> Option<JsonObject> {
        self.state.lock().unwrap().details.clone()
    }

    pub(super) fn fired(&self) -> usize {
        self.state.lock().unwrap().fired
    }

    pub(super) fn fired_with(&self) -> Option<JsonObject> {
        self.state.lock().unwrap().fired_with.clone()
    }

    pub(super) fn assembled(&self) -> Option<(usize, Option<Vec<Json>>, bool)> {
        self.state.lock().unwrap().assembled.clone()
    }

    pub(super) fn dispatched(&self) -> Vec<JsonObject> {
        self.state.lock().unwrap().dispatched.clone()
    }
}

impl RequestLogging for FakeLogging {
    fn call_details(&self, _request_data: &JsonObject) -> Option<JsonObject> {
        let mut state = self.state.lock().unwrap();
        state.events.push("details".to_owned());
        state.details.clone()
    }

    fn edit_call_details(&self, _request_data: &JsonObject, edit: fn(&mut JsonObject)) {
        let mut state = self.state.lock().unwrap();
        state.events.push("edit".to_owned());
        if let Some(details) = &mut state.details {
            edit(details);
        }
    }

    fn deferred_logging_armed(&self, _request_data: &JsonObject) -> bool {
        let mut state = self.state.lock().unwrap();
        state.events.push(format!("armed:{}", state.armed));
        state.armed
    }

    fn fire_deferred_logging(&self, request_data: &JsonObject) {
        let mut state = self.state.lock().unwrap();
        state.fired += 1;
        state
            .events
            .push(format!("fire:{}", disconnect_marked(request_data)));
        state.fired_with = Some(request_data.clone());
    }

    fn baseline_cache_active(&self, _request_data: &JsonObject) -> bool {
        let mut state = self.state.lock().unwrap();
        state.events.push("baseline".to_owned());
        state.baseline_active
    }

    fn invalidate_baseline<'call>(
        &'call self,
        _request_data: &'call JsonObject,
        reason: &'call str,
        completed: bool,
    ) -> CleanupFuture<'call, Result<(), CleanupError>> {
        let mut state = self.state.lock().unwrap();
        state
            .events
            .push(format!("invalidate:{reason}:{completed}"));
        let outcome = state.invalidate.clone().unwrap_or(Ok(()));
        Box::pin(async move { outcome })
    }

    fn assemble_partial<'call>(
        &'call self,
        request_data: &'call JsonObject,
        chunks: &'call [JsonObject],
        messages: Option<&'call [Json]>,
    ) -> CleanupFuture<'call, Result<Option<JsonObject>, CleanupError>> {
        let mut state = self.state.lock().unwrap();
        state.events.push("assemble".to_owned());
        state.assembled = Some((
            chunks.len(),
            messages.map(<[Json]>::to_vec),
            disconnect_marked(request_data),
        ));
        let outcome = state.assemble.clone().unwrap_or(Ok(None));
        Box::pin(async move { outcome })
    }

    fn dispatch_success<'call>(
        &'call self,
        _request_data: &'call JsonObject,
        response: JsonObject,
    ) -> CleanupFuture<'call, Result<(), CleanupError>> {
        let mut state = self.state.lock().unwrap();
        state.events.push("dispatch".to_owned());
        state.dispatched.push(response);
        let outcome = state.dispatch.clone().unwrap_or(Ok(()));
        Box::pin(async move { outcome })
    }
}

#[derive(Default)]
struct SlotState {
    keys: Vec<String>,
    fail: bool,
}

#[derive(Default)]
pub(super) struct FakeSlots {
    state: Mutex<SlotState>,
}

impl FakeSlots {
    pub(super) fn failing() -> Self {
        Self {
            state: Mutex::new(SlotState {
                keys: Vec::new(),
                fail: true,
            }),
        }
    }

    pub(super) fn keys(&self) -> Vec<String> {
        self.state.lock().unwrap().keys.clone()
    }
}

impl SlotRelease<String> for FakeSlots {
    fn release_on_disconnect<'call>(
        &'call self,
        key: &'call String,
    ) -> CleanupFuture<'call, Result<(), CleanupError>> {
        let mut state = self.state.lock().unwrap();
        state.keys.push(key.clone());
        let outcome = if state.fail {
            Err(CleanupError::Port("release failed".to_owned()))
        } else {
            Ok(())
        };
        Box::pin(async move { outcome })
    }
}

struct Snapshot(CollectedStream);

impl StreamSnapshot for Snapshot {
    fn collected(&self) -> CollectedStream {
        self.0.clone()
    }
}

pub(super) fn snapshot(
    chunks: Vec<JsonObject>,
    messages: Option<Vec<Json>>,
    model: Option<&str>,
) -> Arc<dyn StreamSnapshot> {
    Arc::new(Snapshot(CollectedStream {
        chunks,
        messages,
        model: model.map(str::to_owned),
    }))
}

#[derive(Clone, Copy)]
pub(super) enum Ending {
    Completed,
    Disconnected,
    Failed,
}

pub(super) fn cleanup_outcome(ending: Ending) -> CleanupOutcome<String> {
    let (stream_completed, client_disconnected) = match ending {
        Ending::Completed => (true, false),
        Ending::Disconnected => (false, true),
        Ending::Failed => (false, false),
    };
    CleanupOutcome {
        request: None,
        request_data: object(&[("litellm_call_id", text("call-1"))]),
        key: "deployment-1".to_owned(),
        response: None,
        stream_completed,
        client_disconnected,
    }
}

pub(super) fn disconnect_cleanup(
    logging: Arc<FakeLogging>,
    slots: Arc<FakeSlots>,
) -> DisconnectCleanup<String> {
    DisconnectCleanup {
        logging,
        slots,
        tracker: TaskTracker::new(),
        disable_streaming_logging: false,
    }
}

pub(super) fn billable_outcome() -> CleanupOutcome<String> {
    let mut outcome = cleanup_outcome(Ending::Disconnected);
    outcome.response = Some(snapshot(
        vec![object(&[("model", text("alias"))])],
        Some(vec![text("hi")]),
        Some("gpt-4o"),
    ));
    outcome
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).expect("utf-8 log output")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogBuffer, tracing::subscriber::DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
