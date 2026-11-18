#![cfg(test)]

use super::failure::failure_frame;
use super::*;
use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use bytes::Bytes;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};
use std::sync::Arc;
use std::sync::Mutex;

use crate::data_generator_select::AsyncDataGenerator;
use crate::stream_chunks::frames::sse_data_frame;
use crate::stream_chunks::serialize::serialize_chunk;
use crate::stream_chunks::{
    ChunkKind, DONE_FRAME, MAX_RAW_SSE_BUFFER_CHARS, StreamChunk, StreamItem,
};
struct NoHooks;

#[async_trait]
impl StreamHooks<()> for NoHooks {
    fn needs_iterator_wrap(&self) -> bool {
        false
    }
    fn needs_per_chunk_hook(&self) -> bool {
        false
    }
    fn wrap_iterator(&self, items: ItemStream, _: &(), _: &JsonObject) -> ItemStream {
        items
    }
    async fn chunk_hook(
        &self,
        item: StreamItem,
        _: &(),
        _: &JsonObject,
        _: Option<&str>,
    ) -> Result<StreamItem, StreamFailure> {
        Ok(item)
    }
    async fn failure_hook(
        &self,
        _: &(),
        _: &StreamFailure,
        _: &JsonObject,
    ) -> Result<(), StreamFailure> {
        Ok(())
    }
    fn fire_deferred_stream_logging(&self, _: &JsonObject) {
        // No deferred work in this fixture.
    }
}

struct NoErrorState;

impl ErrorStateSource for NoErrorState {
    fn new_state(&self) -> Box<dyn StreamErrorState + Send> {
        Box::new(NoErrorState)
    }
}

struct RecordingErrorSource {
    events: Arc<Mutex<Vec<String>>>,
    failure_frame: Option<String>,
}

impl ErrorStateSource for RecordingErrorSource {
    fn new_state(&self) -> Box<dyn StreamErrorState + Send> {
        self.events
            .lock()
            .expect("events")
            .push("new-state".to_owned());
        Box::new(RecordingErrorState {
            events: self.events.clone(),
            failure_frame: self.failure_frame.clone(),
        })
    }
}

struct RecordingErrorState {
    events: Arc<Mutex<Vec<String>>>,
    failure_frame: Option<String>,
}

impl StreamErrorState for RecordingErrorState {
    fn observe_chunk(&mut self, _: &StreamItem) {
        self.events
            .lock()
            .expect("events")
            .push("observe".to_owned());
    }

    fn mark_emitted(&mut self, frame: String) -> String {
        self.events.lock().expect("events").push("mark".to_owned());
        frame
    }

    fn format_failure(&mut self, _: &StreamFailure) -> Option<String> {
        self.events
            .lock()
            .expect("events")
            .push("format".to_owned());
        self.failure_frame.clone()
    }
}

impl StreamErrorState for NoErrorState {
    fn observe_chunk(&mut self, _: &StreamItem) {
        // No error state is enabled in this fixture.
    }
    fn mark_emitted(&mut self, frame: String) -> String {
        frame
    }
    fn format_failure(&mut self, _: &StreamFailure) -> Option<String> {
        None
    }
}

struct NoCleanup;

impl StreamCleanup<()> for NoCleanup {
    fn finalize(&self, _: CleanupOutcome<()>) {
        // The fixture does not record cleanup.
    }
}

struct RecordingCleanup(Arc<Mutex<Vec<(bool, bool, bool)>>>);

impl StreamCleanup<()> for RecordingCleanup {
    fn finalize(&self, outcome: CleanupOutcome<()>) {
        self.0.lock().expect("cleanup").push((
            outcome.stream_completed,
            outcome.client_disconnected,
            outcome.response.is_some(),
        ));
    }
}

struct EmptySnapshot;

impl StreamSnapshot for EmptySnapshot {
    fn collected(&self) -> CollectedStream {
        CollectedStream::default()
    }
}

struct RecordingHooks {
    events: Arc<Mutex<Vec<String>>>,
    wrap: bool,
    per_chunk: bool,
    chunk_fails: bool,
    failure_hook_fails: bool,
}

#[async_trait]
impl StreamHooks<()> for RecordingHooks {
    fn needs_iterator_wrap(&self) -> bool {
        self.events
            .lock()
            .expect("events")
            .push("needs-wrap".to_owned());
        self.wrap
    }

    fn needs_per_chunk_hook(&self) -> bool {
        self.events
            .lock()
            .expect("events")
            .push("needs-chunk".to_owned());
        self.per_chunk
    }

    fn wrap_iterator(&self, items: ItemStream, _: &(), _: &JsonObject) -> ItemStream {
        self.events.lock().expect("events").push("wrap".to_owned());
        items
    }

    async fn chunk_hook(
        &self,
        item: StreamItem,
        _: &(),
        _: &JsonObject,
        str_so_far: Option<&str>,
    ) -> Result<StreamItem, StreamFailure> {
        self.events
            .lock()
            .expect("events")
            .push(format!("chunk:{}", str_so_far.unwrap_or("<none>")));
        if self.chunk_fails {
            Err(other_failure("guardrail failed"))
        } else {
            Ok(item)
        }
    }

    async fn failure_hook(
        &self,
        _: &(),
        failure: &StreamFailure,
        _: &JsonObject,
    ) -> Result<(), StreamFailure> {
        self.events
            .lock()
            .expect("events")
            .push(format!("failure:{}", failure.message));
        if self.failure_hook_fails {
            Err(other_failure("failure hook failed"))
        } else {
            Ok(())
        }
    }

    fn fire_deferred_stream_logging(&self, _: &JsonObject) {
        self.events
            .lock()
            .expect("events")
            .push("deferred".to_owned());
    }
}

fn other_failure(message: &str) -> StreamFailure {
    StreamFailure {
        message: message.to_owned(),
        error_type: None,
        param: FailureParam::Missing,
        status_code: None,
        kind: FailureKind::Other,
    }
}

#[test]
fn failure_frame_fields() {
    let failure = StreamFailure {
        message: "bad request".to_owned(),
        error_type: None,
        param: FailureParam::Null,
        status_code: Some(422),
        kind: FailureKind::Other,
    };
    let frame = failure_frame(&failure);
    assert!(frame.contains("\"type\":\"None\""));
    assert!(frame.contains("\"param\":null"));
    assert!(frame.contains("\"code\":\"422\""));
    assert!(!frame.contains("[DONE]"));
    let missing_param = StreamFailure {
        message: "missing parameter".to_owned(),
        error_type: None,
        param: FailureParam::Missing,
        status_code: None,
        kind: FailureKind::Other,
    };
    let missing_frame = failure_frame(&missing_param);
    assert!(missing_frame.contains("\"type\":\"None\""));
    assert!(missing_frame.contains("\"param\":\"None\""));
}

#[test]
fn failure_frame_status() {
    let mut failure = StreamFailure {
        message: "No deployments available".to_owned(),
        error_type: Some("Unavailable".to_owned()),
        param: FailureParam::Missing,
        status_code: Some(500),
        kind: FailureKind::Other,
    };
    assert!(failure_frame(&failure).contains("\"type\":\"Unavailable\""));
    assert!(failure_frame(&failure).contains("\"code\":\"429\""));
    failure.message = "Not allowed to access model due to tags configuration".to_owned();
    assert!(failure_frame(&failure).contains("\"code\":\"401\""));
}

#[test]
fn keepalive_source_hidden() {
    let hidden = [("model_id".to_owned(), Json::Text("deployment".to_owned()))]
        .into_iter()
        .collect::<JsonObject>();
    let item = Ok(StreamItem::Chunk(StreamChunk {
        body: JsonObject::new(),
        kind: ChunkKind::Plain,
        hidden: hidden.clone(),
    }));
    assert_eq!(item.hidden_params(), Some(&hidden));
    let failure: Result<StreamItem, StreamFailure> = Err(StreamFailure {
        message: "failure".to_owned(),
        error_type: None,
        param: FailureParam::Missing,
        status_code: None,
        kind: FailureKind::Other,
    });
    assert_eq!(failure.hidden_params(), None);
}

fn collect_frames(
    items: Vec<Result<StreamItem, StreamFailure>>,
    request_data: JsonObject,
) -> Vec<String> {
    collect_with_generator(test_generator(None), items, request_data)
}

fn collect_with_generator(
    generator: SseDataGenerator<()>,
    items: Vec<Result<StreamItem, StreamFailure>>,
    request_data: JsonObject,
) -> Vec<String> {
    let source = StreamSource {
        items: stream::iter(items).boxed(),
        hidden: JsonObject::new(),
        snapshot: None,
    };
    let body = generator.generate(GeneratorInput {
        response: source,
        key: (),
        request_data,
        request: None,
        responses_stream_errors: false,
    });
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(body.collect::<Vec<_>>())
        .into_iter()
        .map(|frame| String::from_utf8(frame.expect("frame").to_vec()).expect("utf8"))
        .collect()
}

fn model_chunk(text: &str) -> StreamItem {
    let delta = [("content".to_owned(), Json::Text(text.to_owned()))]
        .into_iter()
        .collect();
    let choice = [("delta".to_owned(), Json::Object(delta))]
        .into_iter()
        .collect();
    let body = [("choices".to_owned(), Json::List(vec![Json::Object(choice)]))]
        .into_iter()
        .collect();
    StreamItem::Chunk(StreamChunk {
        body,
        kind: ChunkKind::ModelStream,
        hidden: JsonObject::new(),
    })
}

fn fallback_hidden() -> JsonObject {
    let error = [("message".to_owned(), Json::Text("failed".to_owned()))]
        .into_iter()
        .collect();
    let headers = [
        (
            "x-litellm-attempted-fallbacks".to_owned(),
            Json::Number(1.into()),
        ),
        (
            "x-litellm-model-group".to_owned(),
            Json::Text("fallback-model".to_owned()),
        ),
        (
            "x-litellm-fallback-errors".to_owned(),
            Json::List(vec![Json::Object(error)]),
        ),
    ]
    .into_iter()
    .collect();
    [("additional_headers".to_owned(), Json::Object(headers))]
        .into_iter()
        .collect()
}

fn test_generator(
    global_keepalive: Option<crate::keepalive_interval::PingSetting>,
) -> SseDataGenerator<()> {
    let mut generator = generator_with(
        Arc::new(NoHooks),
        Arc::new(NoErrorState),
        Arc::new(NoCleanup),
    );
    generator.global_keepalive = global_keepalive;
    generator
}

fn generator_with(
    hooks: Arc<dyn StreamHooks<()>>,
    error_state: Arc<dyn ErrorStateSource>,
    cleanup: Arc<dyn StreamCleanup<()>>,
) -> SseDataGenerator<()> {
    SseDataGenerator {
        hooks,
        error_state,
        cleanup,
        router: None,
        global_keepalive: None,
        expose_fallback_errors: false,
        is_model_router: |_| false,
    }
}

fn collect_body(body: BoxStream<'static, Result<Bytes, std::io::Error>>) -> Vec<String> {
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(body.collect::<Vec<_>>())
        .into_iter()
        .map(|frame| String::from_utf8(frame.expect("frame").to_vec()).expect("utf8"))
        .collect()
}

fn run_generator(
    generator: SseDataGenerator<()>,
    response: StreamSource,
    request_data: JsonObject,
    responses_stream_errors: bool,
) -> Vec<String> {
    collect_body(generator.generate(GeneratorInput {
        response,
        key: (),
        request_data,
        request: None,
        responses_stream_errors,
    }))
}

mod behavior;
