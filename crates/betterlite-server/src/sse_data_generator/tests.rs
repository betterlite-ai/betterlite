#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::http::request::Parts;
use betterlite_config::sections::settings::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use bytes::Bytes;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};
use serde_json::Number;
use tokio::runtime::Runtime;

use super::types::{
    CleanupOutcome, CollectedStream, ErrorStateSource, FailureKind, FailureParam, ItemStream,
    SseDataGenerator, StreamCleanup, StreamErrorState, StreamFailure, StreamHooks, StreamSnapshot,
    StreamSource,
};
use super::{failure, setup};
use crate::data_generator_select::{AsyncDataGenerator, GeneratorInput};
use crate::stream_chunks::frames::sse_data_frame;
use crate::stream_chunks::serialize::serialize_chunk;
use crate::stream_chunks::{ChunkKind, StreamChunk, StreamItem};
use crate::stream_keepalive::wrapper::KeepaliveSource;
use crate::stream_keepalive::{JsonObject as KeepaliveMap, KeepaliveRouter};

struct NoHooks;

#[async_trait]
impl StreamHooks<String> for NoHooks {
    fn needs_iterator_wrap(&self) -> bool {
        false
    }

    fn needs_per_chunk_hook(&self) -> bool {
        false
    }

    fn wrap_iterator(&self, items: ItemStream, _: &String, _: &JsonObject) -> ItemStream {
        items
    }

    async fn chunk_hook(
        &self,
        item: StreamItem,
        _: &String,
        _: &JsonObject,
        _: Option<&str>,
    ) -> Result<StreamItem, StreamFailure> {
        Ok(item)
    }

    async fn failure_hook(
        &self,
        _: &String,
        _: &StreamFailure,
        _: &JsonObject,
    ) -> Result<(), StreamFailure> {
        Ok(())
    }

    fn fire_deferred_stream_logging(&self, _: &JsonObject) {}
}

struct RecordingHooks {
    events: Arc<Mutex<Vec<String>>>,
    wrap: AtomicBool,
    per_chunk: AtomicBool,
    chunk_fails: AtomicBool,
    rewrite: AtomicBool,
    failure_fails: AtomicBool,
}

impl RecordingHooks {
    fn record(&self, event: &str) {
        self.events.lock().expect("events").push(event.to_owned());
    }

    fn set_wrap(&self, on: bool) {
        self.wrap.store(on, Ordering::Relaxed);
    }

    fn set_per_chunk(&self, on: bool) {
        self.per_chunk.store(on, Ordering::Relaxed);
    }

    fn set_chunk_fails(&self, on: bool) {
        self.chunk_fails.store(on, Ordering::Relaxed);
    }

    fn set_failure_fails(&self, on: bool) {
        self.failure_fails.store(on, Ordering::Relaxed);
    }
}

#[async_trait]
impl StreamHooks<String> for RecordingHooks {
    fn needs_iterator_wrap(&self) -> bool {
        self.record("needs-wrap");
        self.wrap.load(Ordering::Relaxed)
    }

    fn needs_per_chunk_hook(&self) -> bool {
        self.record("needs-chunk");
        self.per_chunk.load(Ordering::Relaxed)
    }

    fn wrap_iterator(&self, items: ItemStream, _: &String, _: &JsonObject) -> ItemStream {
        self.record("wrap");
        stream::iter([Ok(StreamItem::Text(String::from("wrapped")))])
            .chain(items)
            .boxed()
    }

    async fn chunk_hook(
        &self,
        item: StreamItem,
        _: &String,
        _: &JsonObject,
        str_so_far: Option<&str>,
    ) -> Result<StreamItem, StreamFailure> {
        self.record(&format!("chunk:{}", str_so_far.unwrap_or("<none>")));
        if self.chunk_fails.load(Ordering::Relaxed) {
            return Err(other_failure("guardrail failed"));
        }
        if self.rewrite.load(Ordering::Relaxed) {
            return Ok(chat_chunk("hooked"));
        }
        Ok(item)
    }

    async fn failure_hook(
        &self,
        _: &String,
        failure: &StreamFailure,
        _: &JsonObject,
    ) -> Result<(), StreamFailure> {
        self.record(&format!("failure:{}", failure.message));
        if self.failure_fails.load(Ordering::Relaxed) {
            return Err(other_failure("failure hook failed"));
        }
        Ok(())
    }

    fn fire_deferred_stream_logging(&self, _: &JsonObject) {
        self.record("deferred");
    }
}

struct NoErrorState;

impl ErrorStateSource for NoErrorState {
    fn new_state(&self) -> Box<dyn StreamErrorState + Send> {
        Box::new(NoState)
    }
}

struct NoState;

impl StreamErrorState for NoState {
    fn observe_chunk(&mut self, _: &StreamItem) {}

    fn mark_emitted(&mut self, frame: String) -> String {
        frame
    }

    fn format_failure(&mut self, _: &StreamFailure) -> Option<String> {
        None
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
            events: Arc::clone(&self.events),
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

struct NoCleanup;

impl StreamCleanup<String> for NoCleanup {
    fn finalize(&self, _: CleanupOutcome<String>) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndState {
    Completed,
    Disconnected,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Outcome {
    key: String,
    state: EndState,
    has_request: bool,
    has_snapshot: bool,
}

struct RecordingCleanup(Arc<Mutex<Vec<Outcome>>>);

impl StreamCleanup<String> for RecordingCleanup {
    fn finalize(&self, outcome: CleanupOutcome<String>) {
        assert!(!outcome.stream_completed || !outcome.client_disconnected);
        let state = match (outcome.stream_completed, outcome.client_disconnected) {
            (true, _) => EndState::Completed,
            (false, true) => EndState::Disconnected,
            (false, false) => EndState::Aborted,
        };
        self.0.lock().expect("cleanup").push(Outcome {
            key: outcome.key,
            state,
            has_request: outcome.request.is_some(),
            has_snapshot: outcome.response.is_some(),
        });
    }
}

struct EmptySnapshot;

impl StreamSnapshot for EmptySnapshot {
    fn collected(&self) -> CollectedStream {
        CollectedStream::default()
    }
}

struct TestRouter;

impl KeepaliveRouter for TestRouter {
    fn deployment_params(&self, model_id: &str) -> Option<KeepaliveMap> {
        let seconds = match model_id {
            "fast" => 1_i64,
            _ => 0_i64,
        };
        Some(KeepaliveMap::from([(
            "keepalive_seconds".to_owned(),
            JsonValue::Number(Number::from(seconds)),
        )]))
    }

    fn group_deployments(&self, _: Option<&str>) -> Vec<JsonValue> {
        Vec::new()
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

fn object(pairs: &[(&str, Json)]) -> JsonObject {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn request(pairs: &[(&str, JsonValue)]) -> BTreeMap<String, JsonValue> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn choices_body(container: &str, text: &str) -> JsonObject {
    let message = object(&[("content", Json::Text(text.to_owned()))]);
    let choice = object(&[(container, Json::Object(message))]);
    object(&[("choices", Json::List(vec![Json::Object(choice)]))])
}

fn expected_chunk(text: &str, model: Option<&str>) -> StreamChunk {
    let mut body = choices_body("delta", text);
    if let Some(model) = model {
        body.insert("model".to_owned(), Json::Text(model.to_owned()));
    }
    StreamChunk {
        body,
        kind: ChunkKind::ModelStream,
        hidden: JsonObject::new(),
    }
}

fn chat_chunk(text: &str) -> StreamItem {
    StreamItem::Chunk(expected_chunk(text, None))
}

fn full_chunk(text: &str) -> StreamItem {
    StreamItem::Chunk(StreamChunk {
        body: choices_body("message", text),
        kind: ChunkKind::ModelFull,
        hidden: JsonObject::new(),
    })
}

fn plain_chunk() -> StreamItem {
    StreamItem::Chunk(StreamChunk {
        body: object(&[("object", Json::Text("plain".to_owned()))]),
        kind: ChunkKind::Plain,
        hidden: JsonObject::new(),
    })
}

fn usage_artifact() -> StreamItem {
    StreamItem::Chunk(StreamChunk {
        body: object(&[
            ("choices", Json::List(Vec::new())),
            (
                "usage",
                Json::Object(object(&[(
                    "prompt_tokens",
                    Json::Number(Number::from(1_i64)),
                )])),
            ),
        ]),
        kind: ChunkKind::ModelStream,
        hidden: JsonObject::new(),
    })
}

fn fallback_hidden(model: Option<&str>, errors: &[&str]) -> JsonObject {
    let mut headers = object(&[(
        "x-litellm-attempted-fallbacks",
        Json::Number(Number::from(1_i64)),
    )]);
    if let Some(model) = model {
        headers.insert(
            "x-litellm-model-group".to_owned(),
            Json::Text(model.to_owned()),
        );
    }
    if !errors.is_empty() {
        let encoded = Json::List(
            errors
                .iter()
                .map(|message| {
                    Json::Object(object(&[("message", Json::Text((*message).to_owned()))]))
                })
                .collect(),
        );
        headers.insert(
            "x-litellm-fallback-errors".to_owned(),
            Json::Text(serde_json::to_string(&encoded).expect("errors")),
        );
    }
    object(&[("additional_headers", Json::Object(headers))])
}

fn with_hidden(item: StreamItem, hidden: JsonObject) -> StreamItem {
    match item {
        StreamItem::Chunk(mut chunk) => {
            chunk.hidden = hidden;
            StreamItem::Chunk(chunk)
        }
        StreamItem::Bytes(bytes) => StreamItem::Bytes(bytes),
        StreamItem::Text(text) => StreamItem::Text(text),
    }
}

fn source(items: Vec<Result<StreamItem, StreamFailure>>, hidden: JsonObject) -> StreamSource {
    StreamSource {
        items: stream::iter(items).boxed(),
        hidden,
        snapshot: None,
    }
}

fn quiet_source(items: Vec<Result<StreamItem, StreamFailure>>, hidden: JsonObject) -> StreamSource {
    StreamSource {
        items: stream::iter(items)
            .chain(stream::pending::<Result<StreamItem, StreamFailure>>())
            .boxed(),
        hidden,
        snapshot: None,
    }
}

fn snapshot_handle() -> Arc<dyn StreamSnapshot> {
    Arc::new(EmptySnapshot)
}

fn plain_generator() -> SseDataGenerator<String> {
    let hooks: Arc<dyn StreamHooks<String>> = Arc::new(NoHooks);
    let error_state: Arc<dyn ErrorStateSource> = Arc::new(NoErrorState);
    let cleanup: Arc<dyn StreamCleanup<String>> = Arc::new(NoCleanup);
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

fn recording_generator(
    events: Arc<Mutex<Vec<String>>>,
    outcomes: Arc<Mutex<Vec<Outcome>>>,
    failure_frame: Option<String>,
) -> (SseDataGenerator<String>, Arc<RecordingHooks>) {
    let hooks = Arc::new(RecordingHooks {
        events: Arc::clone(&events),
        wrap: AtomicBool::new(false),
        per_chunk: AtomicBool::new(false),
        chunk_fails: AtomicBool::new(false),
        rewrite: AtomicBool::new(false),
        failure_fails: AtomicBool::new(false),
    });
    let hooks_ref: Arc<dyn StreamHooks<String>> = Arc::<RecordingHooks>::clone(&hooks);
    let error_state: Arc<dyn ErrorStateSource> = Arc::new(RecordingErrorSource {
        events,
        failure_frame,
    });
    let cleanup: Arc<dyn StreamCleanup<String>> = Arc::new(RecordingCleanup(outcomes));
    let generator = SseDataGenerator {
        hooks: hooks_ref,
        error_state,
        cleanup,
        router: None,
        global_keepalive: None,
        expose_fallback_errors: false,
        is_model_router: |_| false,
    };
    (generator, hooks)
}

fn body_of(
    generator: &SseDataGenerator<String>,
    source: StreamSource,
    request_data: BTreeMap<String, JsonValue>,
    responses_stream_errors: bool,
) -> BoxStream<'static, Result<Bytes, io::Error>> {
    body_with_request(
        generator,
        source,
        request_data,
        None,
        responses_stream_errors,
    )
}

fn body_with_request(
    generator: &SseDataGenerator<String>,
    source: StreamSource,
    request_data: BTreeMap<String, JsonValue>,
    request: Option<Parts>,
    responses_stream_errors: bool,
) -> BoxStream<'static, Result<Bytes, io::Error>> {
    generator.generate(GeneratorInput {
        response: source,
        key: String::from("key-1"),
        request_data,
        request,
        responses_stream_errors,
    })
}

fn collect(
    generator: &SseDataGenerator<String>,
    source: StreamSource,
    request_data: BTreeMap<String, JsonValue>,
    responses_stream_errors: bool,
) -> Vec<Result<Bytes, io::Error>> {
    let body = body_of(generator, source, request_data, responses_stream_errors);
    Runtime::new()
        .expect("runtime")
        .block_on(body.collect::<Vec<_>>())
}

fn frames(
    generator: &SseDataGenerator<String>,
    source: StreamSource,
    request_data: BTreeMap<String, JsonValue>,
    responses_stream_errors: bool,
) -> Vec<String> {
    collect(generator, source, request_data, responses_stream_errors)
        .into_iter()
        .map(|frame| String::from_utf8(frame.expect("frame").to_vec()).expect("utf8"))
        .collect()
}

fn recorded(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    log.lock().expect("events").clone()
}

fn recorded_outcomes(outcomes: &Arc<Mutex<Vec<Outcome>>>) -> Vec<Outcome> {
    outcomes.lock().expect("cleanup").clone()
}

fn count(log: &[String], event: &str) -> usize {
    log.iter()
        .filter(|recorded| recorded.as_str() == event)
        .count()
}

fn count_prefix(log: &[String], prefix: &str) -> usize {
    log.iter()
        .filter(|recorded| recorded.starts_with(prefix))
        .count()
}

fn chunk_frame(chunk: &StreamChunk) -> String {
    sse_data_frame(&serialize_chunk(chunk))
}

#[test]
fn keepalive_source_hidden() {
    let hidden = object(&[("model_id", Json::Text("deployment".to_owned()))]);
    let item: Result<StreamItem, StreamFailure> = Ok(with_hidden(chat_chunk("x"), hidden.clone()));
    assert_eq!(item.stream_hidden_params(), Some(&hidden));
    let failure: Result<StreamItem, StreamFailure> = Err(other_failure("failure"));
    assert_eq!(failure.stream_hidden_params(), None);
}

#[test]
fn request_converts_every_value() {
    let data = request(&[
        ("null", JsonValue::Null),
        ("bool", JsonValue::from(true)),
        ("number", JsonValue::from(7_i64)),
        ("text", JsonValue::from("value")),
        (
            "list",
            serde_json::from_str::<JsonValue>("[1, \"two\"]").expect("list"),
        ),
        (
            "object",
            serde_json::from_str::<JsonValue>(r#"{"inner": {"deep": null}}"#).expect("object"),
        ),
    ]);
    let converted = setup::request_object(&data);
    assert_eq!(
        serde_json::to_string(&converted).expect("converted"),
        r#"{"bool":true,"list":[1,"two"],"null":null,"number":7,"object":{"inner":{"deep":null}},"text":"value"}"#
    );
}

mod behavior;
mod cleanup_tests;
mod failures;
mod keepalive;

#[test]
fn timestamp_is_current() {
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("epoch")
        .as_secs();
    let timestamp = failure::unix_timestamp();
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("epoch")
        .as_secs();
    assert!((before..=after).contains(&u64::try_from(timestamp).expect("positive timestamp")));
}

struct LegacyItem;

impl KeepaliveSource for LegacyItem {
    fn hidden_params(&self) -> Option<&KeepaliveMap> {
        None
    }
}

#[test]
fn keepalive_default_metadata() {
    assert_eq!(LegacyItem.stream_hidden_params(), None);
    let failure: Result<StreamItem, StreamFailure> = Err(other_failure("failure"));
    assert_eq!(failure.hidden_params(), None);
}

#[test]
fn keepalive_converts_every_value() {
    let data = object(&[
        ("null", Json::Null),
        ("bool", Json::Bool(true)),
        ("number", Json::Number(Number::from(7))),
        ("text", Json::Text(String::from("value"))),
        ("list", Json::List(vec![Json::Bool(false)])),
        ("object", Json::Object(object(&[("inner", Json::Null)]))),
    ]);
    let converted = crate::stream_keepalive::wrapper::keepalive_object(&data);
    assert_eq!(setup::request_object(&converted), data);
}
