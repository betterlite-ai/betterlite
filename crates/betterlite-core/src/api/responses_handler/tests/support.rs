use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_context::non_default_params::Params;
use betterlite_interactions::types::json::Object as InteractionObject;
use betterlite_interactions::types::response::InteractionsStreamChunk;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse as UpstreamEvent;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use jiff::tz::TimeZone;
use thiserror::Error;
use uuid::Uuid;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_handler::async_iterator::InteractionsAsyncIterator;
use crate::api::responses_handler::dispatch::{AsyncHandlerRequest, HandlerRequest};
use crate::api::responses_handler::operation::{AsyncResponsesOperation, ResponsesOperation};
use crate::api::responses_handler::state::{InteractionsStreamState, IteratorSetup};
use crate::api::responses_handler::streams::{
    AsyncResponsesAnswer, AsyncResponsesEventStream, SyncResponsesAnswer, SyncResponsesEventStream,
};
use crate::api::responses_handler::sync_iterator::InteractionsSyncIterator;
use crate::api::responses_transform::request::ResponsesRequest;

#[derive(Debug, Error)]
pub(super) enum FakeError {
    #[error("the responses stream failed")]
    Stream,
    #[error("the responses operation failed")]
    Operation,
}

pub(super) struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(7)
    }
}

pub(super) type Events = Vec<Result<UpstreamEvent, FakeError>>;
pub(super) type Pulls = Arc<AtomicUsize>;
pub(super) type SyncIterator = InteractionsSyncIterator<FakeError>;
pub(super) type AsyncIterator = InteractionsAsyncIterator<FakeError>;
type SyncAnswer = Result<SyncResponsesAnswer<FakeError>, FakeError>;
type AsyncAnswer = Result<AsyncResponsesAnswer<FakeError>, FakeError>;

pub(super) fn fallback_id() -> String {
    format!("interaction_{}", Uuid::from_u128(7).simple())
}

pub(super) fn params(source: &str) -> Params {
    serde_json::from_str(source).expect("the params fixture should parse")
}

pub(super) fn value(source: &str) -> JsonValue {
    serde_json::from_str(source).expect("the value fixture should parse")
}

pub(super) fn object(source: &str) -> InteractionObject {
    serde_json::from_str(source).expect("the object fixture should parse")
}

pub(super) fn expected_chunk(source: &str) -> InteractionsStreamChunk {
    serde_json::from_str(source).expect("the expected event fixture should parse")
}

pub(super) fn response(source: &str) -> ResponsesApiResponse {
    serde_json::from_str(source).expect("the response fixture should parse")
}

pub(super) fn chunk(source: &str) -> UpstreamEvent {
    serde_json::from_str(source).expect("the chunk fixture should parse")
}

pub(super) fn delta_chunk(item_id: &str, text: &str) -> UpstreamEvent {
    chunk(&format!(
        r#"{{"type": "response.output_text.delta", "item_id": "{item_id}", "output_index": 0, "content_index": 0, "delta": "{text}"}}"#
    ))
}

pub(super) fn created_chunk(response_id: &str) -> UpstreamEvent {
    envelope("response.created", response_id)
}

pub(super) fn in_progress_chunk(response_id: &str) -> UpstreamEvent {
    envelope("response.in_progress", response_id)
}

pub(super) fn completed_chunk(response_id: &str) -> UpstreamEvent {
    envelope("response.completed", response_id)
}

pub(super) fn done_chunk() -> UpstreamEvent {
    chunk(
        r#"{"type": "response.output_text.done", "item_id": "item_1", "output_index": 0, "content_index": 0, "text": "hello"}"#,
    )
}

pub(super) fn unknown_chunk() -> UpstreamEvent {
    chunk(r#"{"type": "response.something_else", "value": 1}"#)
}

fn envelope(event_type: &str, response_id: &str) -> UpstreamEvent {
    chunk(&format!(
        r#"{{"type": "{event_type}", "response": {{"id": "{response_id}", "created_at": 1700000000, "output": []}}}}"#
    ))
}

// One fixture of the handler and iterator arguments; each builder borrows the parts its entry point takes.
pub(super) struct Fixture {
    pub(super) model: String,
    pub(super) input: Option<JsonValue>,
    pub(super) optional_params: Params,
    pub(super) custom_llm_provider: Option<String>,
    pub(super) stream: Option<bool>,
    pub(super) extras: Params,
    pub(super) metadata: Params,
    pub(super) zone: TimeZone,
    pub(super) ids: FixedIds,
    pub(super) limits: ApiLimits,
}

impl Fixture {
    pub(super) fn new(model: &str) -> Self {
        Self {
            model: model.to_owned(),
            input: None,
            optional_params: Params::new(),
            custom_llm_provider: None,
            stream: None,
            extras: Params::new(),
            metadata: Params::new(),
            zone: TimeZone::UTC,
            ids: FixedIds,
            limits: ApiLimits::default(),
        }
    }

    pub(super) fn setup(&self) -> IteratorSetup<'_> {
        IteratorSetup {
            model: &self.model,
            request_input: self.input.as_ref(),
            optional_params: &self.optional_params,
            custom_llm_provider: self.custom_llm_provider.as_deref(),
            litellm_metadata: &self.metadata,
            ids: &self.ids,
            limits: self.limits,
        }
    }

    pub(super) fn request(&self) -> HandlerRequest<'_> {
        let setup = self.setup();
        let stream = self.stream;
        let extras = &self.extras;
        let zone = &self.zone;
        HandlerRequest {
            setup,
            stream,
            extras,
            zone,
        }
    }

    pub(super) fn async_request<'call>(
        &'call self,
        responses_request: &'call ResponsesRequest,
    ) -> AsyncHandlerRequest<'call> {
        AsyncHandlerRequest {
            responses_request,
            setup: self.setup(),
            zone: &self.zone,
        }
    }
}

struct FakeStream {
    events: VecDeque<Result<UpstreamEvent, FakeError>>,
    pulls: Pulls,
}

impl FakeStream {
    fn new(events: Events, pulls: Pulls) -> Self {
        Self {
            events: VecDeque::from(events),
            pulls,
        }
    }

    fn pull(&mut self) -> Option<Result<UpstreamEvent, FakeError>> {
        self.pulls.fetch_add(1, Ordering::SeqCst);
        self.events.pop_front()
    }
}

impl SyncResponsesEventStream for FakeStream {
    type Error = FakeError;

    fn next_event(&mut self) -> Option<Result<UpstreamEvent, FakeError>> {
        self.pull()
    }
}

#[async_trait]
impl AsyncResponsesEventStream for FakeStream {
    type Error = FakeError;

    async fn next_event(&mut self) -> Option<Result<UpstreamEvent, FakeError>> {
        self.pull()
    }
}

pub(super) fn sync_stream(events: Events) -> Box<dyn SyncResponsesEventStream<Error = FakeError>> {
    Box::new(FakeStream::new(events, Arc::new(AtomicUsize::new(0))))
}

pub(super) fn async_stream(
    events: Events,
) -> Box<dyn AsyncResponsesEventStream<Error = FakeError>> {
    Box::new(FakeStream::new(events, Arc::new(AtomicUsize::new(0))))
}

trait FakeIterator: Sized {
    fn from_fake(stream: FakeStream, setup: &IteratorSetup<'_>) -> Self;
}

impl FakeIterator for SyncIterator {
    fn from_fake(stream: FakeStream, setup: &IteratorSetup<'_>) -> Self {
        Self::new(Box::new(stream), setup)
    }
}

impl FakeIterator for AsyncIterator {
    fn from_fake(stream: FakeStream, setup: &IteratorSetup<'_>) -> Self {
        Self::new(Box::new(stream), setup)
    }
}

fn fake_iterator<I>(fixture: &Fixture, events: Events) -> (I, Pulls)
where
    I: FakeIterator,
{
    let pulls = Arc::new(AtomicUsize::new(0));
    let stream = FakeStream::new(events, Arc::clone(&pulls));
    (I::from_fake(stream, &fixture.setup()), pulls)
}

pub(super) fn sync_iterator(fixture: &Fixture, events: Events) -> (SyncIterator, Pulls) {
    fake_iterator::<SyncIterator>(fixture, events)
}

pub(super) fn async_iterator(fixture: &Fixture, events: Events) -> (AsyncIterator, Pulls) {
    fake_iterator::<AsyncIterator>(fixture, events)
}

#[derive(Clone)]
pub(super) struct RecordedRequest {
    pub(super) model: String,
    pub(super) input: Option<JsonValue>,
    pub(super) instructions: Option<String>,
    pub(super) stream: Option<bool>,
    pub(super) store: Option<bool>,
    pub(super) passthrough: Params,
}

#[derive(Default)]
pub(super) struct RequestRecorder {
    recorded: Mutex<Option<RecordedRequest>>,
}

impl RequestRecorder {
    fn record(&self, request: &ResponsesRequest) {
        let recorded = RecordedRequest {
            model: request.model.clone(),
            input: request.input.clone(),
            instructions: request.instructions.clone(),
            stream: request.stream,
            store: request.store,
            passthrough: request.passthrough.clone(),
        };
        *self.recorded.lock().unwrap_or_else(PoisonError::into_inner) = Some(recorded);
    }

    pub(super) fn recorded(&self) -> Option<RecordedRequest> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn take_answer<T>(answer: &Mutex<Option<T>>) -> T {
    answer
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .expect("the handler calls the operation exactly once")
}

pub(super) struct RecordingOperation {
    answer: Mutex<Option<SyncAnswer>>,
    pub(super) recorder: RequestRecorder,
}

impl RecordingOperation {
    pub(super) fn new(answer: SyncAnswer) -> Self {
        Self {
            answer: Mutex::new(Some(answer)),
            recorder: RequestRecorder::default(),
        }
    }
}

impl ResponsesOperation for RecordingOperation {
    type Context = ();
    type Error = FakeError;

    fn responses(&self, request: &ResponsesRequest, _context: ()) -> SyncAnswer {
        self.recorder.record(request);
        take_answer(&self.answer)
    }
}

pub(super) struct AsyncRecordingOperation {
    answer: Mutex<Option<AsyncAnswer>>,
    pub(super) recorder: RequestRecorder,
}

impl AsyncRecordingOperation {
    pub(super) fn new(answer: AsyncAnswer) -> Self {
        Self {
            answer: Mutex::new(Some(answer)),
            recorder: RequestRecorder::default(),
        }
    }
}

#[async_trait]
impl AsyncResponsesOperation for AsyncRecordingOperation {
    type Context = ();
    type Error = FakeError;

    async fn aresponses(&self, request: &ResponsesRequest, _context: ()) -> AsyncAnswer {
        self.recorder.record(request);
        take_answer(&self.answer)
    }
}

pub(super) const MESSAGE_RESPONSE: &str = r#"{
    "id": "resp_1",
    "created_at": 1700000000,
    "model": "resp-model",
    "status": "completed",
    "output": [
        {
            "type": "message",
            "id": "msg_1",
            "status": "completed",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "hello", "annotations": []}]
        }
    ]
}"#;

pub(super) fn interaction_state(model: &str) -> InteractionsStreamState {
    let fixture = Fixture::new(model);
    InteractionsStreamState::new(&fixture.setup())
}
