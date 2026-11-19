#![cfg(test)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use bytes::Bytes;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};
use tokio::runtime::Runtime;

use super::call::{SharedCall, SharedSource, StreamAbort};
use super::framing::ChunkFraming;
use super::generator::{DEFAULT_LIMITS, SharedStreamGenerator};
use super::ports::{BufferedOutput, CostInjection, ReservationRelease};
use super::sse::Emitted;
use super::tail::{ANTHROPIC_PING_BYTES, ANTHROPIC_PING_CHUNK};
use crate::keepalive_interval::PingSetting;
use crate::sse_data_generator::types::{
    CleanupOutcome, CollectedStream, FailureKind, FailureParam, FailureType, ItemStream,
    StreamCleanup, StreamFailure, StreamHooks, StreamSnapshot,
};
use crate::stream_chunks::{ChunkKind, StreamChunk, StreamItem};

struct HookConfig {
    per_chunk: bool,
    wrap_prefix: bool,
    rewrite: bool,
    chunk_failure: Option<StreamFailure>,
    replacement: Option<StreamFailure>,
}

impl HookConfig {
    fn plain() -> Self {
        Self {
            per_chunk: false,
            wrap_prefix: false,
            rewrite: false,
            chunk_failure: None,
            replacement: None,
        }
    }
}

struct RecordingHooks {
    events: Arc<Mutex<Vec<String>>>,
    config: HookConfig,
}

impl RecordingHooks {
    fn record(&self, event: &str) {
        self.events.lock().expect("events").push(event.to_owned());
    }
}

#[async_trait]
impl StreamHooks<String> for RecordingHooks {
    fn needs_iterator_wrap(&self) -> bool {
        false
    }

    fn needs_per_chunk_hook(&self) -> bool {
        self.config.per_chunk
    }

    fn wrap_iterator(&self, items: ItemStream, _key: &String, request: &JsonObject) -> ItemStream {
        self.record(&format!("wrap:{}", text_at(request, "model")));
        if self.config.wrap_prefix {
            return stream::iter([Ok(StreamItem::Text(String::from("wrapped")))])
                .chain(items)
                .boxed();
        }
        items
    }

    async fn chunk_hook(
        &self,
        item: StreamItem,
        _key: &String,
        _request_data: &JsonObject,
        str_so_far: Option<&str>,
    ) -> Result<StreamItem, StreamFailure> {
        self.record(&format!("chunk:{}", str_so_far.unwrap_or("<none>")));
        if let Some(failure) = &self.config.chunk_failure {
            return Err(failure.clone());
        }
        if self.config.rewrite {
            return Ok(StreamItem::Text(String::from("hooked")));
        }
        Ok(item)
    }

    async fn failure_hook(
        &self,
        _key: &String,
        failure: &StreamFailure,
        _request_data: &JsonObject,
    ) -> Result<(), StreamFailure> {
        self.record(&format!("failure:{}", failure.message));
        match &self.config.replacement {
            Some(replacement) => Err(replacement.clone()),
            None => Ok(()),
        }
    }

    fn fire_deferred_stream_logging(&self, _request_data: &JsonObject) {
        self.record("deferred");
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Outcome {
    key: String,
    stream_completed: bool,
    client_disconnected: bool,
    snapshot: Option<String>,
}

struct RecordingCleanup(Arc<Mutex<Vec<Outcome>>>);

impl StreamCleanup<String> for RecordingCleanup {
    fn finalize(&self, outcome: CleanupOutcome<String>) {
        let snapshot = outcome
            .response
            .as_ref()
            .and_then(|response| response.collected().model);
        self.0.lock().expect("cleanup").push(Outcome {
            key: outcome.key,
            stream_completed: outcome.stream_completed,
            client_disconnected: outcome.client_disconnected,
            snapshot,
        });
    }
}

struct RecordingRelease(Arc<Mutex<Vec<String>>>);

impl ReservationRelease<String> for RecordingRelease {
    fn release_on_cancel(&self, key: &String) {
        self.0.lock().expect("release").push(key.clone());
    }
}

struct RecordingCost(Arc<Mutex<Vec<String>>>);

impl CostInjection for RecordingCost {
    fn inject(&self, item: StreamItem, model_name: &str) -> StreamItem {
        self.0.lock().expect("cost").push(model_name.to_owned());
        match item {
            StreamItem::Text(text) => StreamItem::Text(format!("cost {text}")),
            other => other,
        }
    }
}

struct FixedBuffered(bool);

impl BufferedOutput for FixedBuffered {
    fn has_buffered_provider_output(&self) -> bool {
        self.0
    }
}

struct TestSnapshot;

impl StreamSnapshot for TestSnapshot {
    fn collected(&self) -> CollectedStream {
        CollectedStream {
            chunks: Vec::new(),
            messages: None,
            model: Some(String::from("snap")),
        }
    }
}

struct Rig {
    events: Arc<Mutex<Vec<String>>>,
    outcomes: Arc<Mutex<Vec<Outcome>>>,
    releases: Arc<Mutex<Vec<String>>>,
    costs: Arc<Mutex<Vec<String>>>,
    generator: SharedStreamGenerator<String, RecordingHooks>,
}

impl Rig {
    fn new(config: HookConfig) -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let releases = Arc::new(Mutex::new(Vec::new()));
        let costs = Arc::new(Mutex::new(Vec::new()));
        let hooks = Arc::new(RecordingHooks {
            events: Arc::clone(&events),
            config,
        });
        let generator = SharedStreamGenerator {
            hooks,
            reservation: Arc::new(RecordingRelease(Arc::clone(&releases))),
            cleanup: Arc::new(RecordingCleanup(Arc::clone(&outcomes))),
            cost: None,
            redactor: None,
            limits: DEFAULT_LIMITS,
            anthropic_ping: Some(PingSetting::Seconds(15.0)),
        };
        Self {
            events,
            outcomes,
            releases,
            costs,
            generator,
        }
    }

    fn with_cost(mut self) -> Self {
        let cost = Arc::new(RecordingCost(Arc::clone(&self.costs)));
        self.generator.cost = Some(cost);
        self
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().expect("events").clone()
    }

    fn outcomes(&self) -> Vec<Outcome> {
        self.outcomes.lock().expect("outcomes").clone()
    }

    fn releases(&self) -> Vec<String> {
        self.releases.lock().expect("releases").clone()
    }

    fn costs(&self) -> Vec<String> {
        self.costs.lock().expect("costs").clone()
    }
}

fn object(pairs: &[(&str, Json)]) -> JsonObject {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn text_at(data: &JsonObject, key: &str) -> String {
    match data.get(key) {
        Some(Json::Text(text)) => text.clone(),
        Some(_) | None => String::new(),
    }
}

fn call(items: Vec<Result<StreamItem, StreamFailure>>) -> SharedCall<String> {
    SharedCall {
        source: SharedSource {
            items: stream::iter(items).boxed(),
            snapshot: Some(Arc::new(TestSnapshot)),
            buffered: None,
        },
        key: String::from("key-1"),
        request_data: object(&[("model", Json::Text(String::from("requested")))]),
        request: None,
    }
}

fn pending_call() -> SharedCall<String> {
    SharedCall {
        source: SharedSource {
            items: stream::pending::<Result<StreamItem, StreamFailure>>().boxed(),
            snapshot: None,
            buffered: None,
        },
        key: String::from("key-1"),
        request_data: object(&[]),
        request: None,
    }
}

fn text_chunk(text: &str) -> StreamItem {
    StreamItem::Text(text.to_owned())
}

fn event_chunk(pairs: &[(&str, Json)]) -> StreamItem {
    StreamItem::Chunk(StreamChunk {
        body: object(pairs),
        kind: ChunkKind::Event,
        hidden: JsonObject::new(),
    })
}

fn model_chunk(text: &str) -> StreamItem {
    let message = object(&[("content", Json::Text(text.to_owned()))]);
    let choice = object(&[("delta", Json::Object(message))]);
    StreamItem::Chunk(StreamChunk {
        body: object(&[("choices", Json::List(vec![Json::Object(choice)]))]),
        kind: ChunkKind::ModelStream,
        hidden: JsonObject::new(),
    })
}

fn other_failure(message: &str) -> StreamFailure {
    StreamFailure {
        message: message.to_owned(),
        error_type: FailureType::Missing,
        param: FailureParam::Missing,
        status_code: None,
        code: None,
        body: None,
        kind: FailureKind::Other,
    }
}

fn status_failure(message: &str, status: u16) -> StreamFailure {
    let mut failure = other_failure(message);
    failure.status_code = Some(status);
    failure
}

fn drive<Out>(body: BoxStream<'static, Result<Out, StreamAbort>>) -> Vec<Result<Out, StreamAbort>>
where
    Out: Send + 'static,
{
    Runtime::new()
        .expect("runtime")
        .block_on(body.collect::<Vec<_>>())
}

fn text_of(emitted: Emitted) -> String {
    match emitted {
        Emitted::Frame(bytes) => String::from_utf8(bytes.to_vec()).expect("utf8"),
        Emitted::AnthropicError(frame) => frame.body,
    }
}

fn texts(body: BoxStream<'static, Result<Emitted, StreamAbort>>) -> Vec<String> {
    drive(body)
        .into_iter()
        .map(|item| text_of(item.expect("frame")))
        .collect()
}

fn abort_of(results: &[Result<Emitted, StreamAbort>], index: usize) -> StreamAbort {
    match results.get(index) {
        Some(Err(abort)) => abort.clone(),
        Some(Ok(_)) => panic!("expected an abort"),
        None => panic!("no item at that index"),
    }
}

#[test]
fn wrap_iterator_always_runs() {
    let mut config = HookConfig::plain();
    config.wrap_prefix = true;
    let rig = Rig::new(config);
    let frames = texts(
        rig.generator
            .sse_data_generator(call(vec![Ok(text_chunk("body"))]), None),
    );
    assert_eq!(frames, vec![String::from("wrapped"), String::from("body")]);
    assert_eq!(rig.events(), vec![String::from("wrap:requested")]);
}

#[test]
fn starts_only_on_first_poll() {
    let rig = Rig::new(HookConfig::plain());
    let mut body = rig
        .generator
        .sse_data_generator(call(vec![Ok(text_chunk("x"))]), None);
    assert_eq!(rig.events().len(), 0);
    let first = Runtime::new().expect("runtime").block_on(body.next());
    assert_eq!(text_of(first.expect("frame").expect("ok")), "x");
    assert_eq!(rig.events(), vec![String::from("wrap:requested")]);
}

#[test]
fn fast_path_skips_chunk_hook() {
    let rig = Rig::new(HookConfig::plain());
    let frames = texts(
        rig.generator
            .sse_data_generator(call(vec![Ok(text_chunk("x")), Ok(text_chunk("y"))]), None),
    );
    assert_eq!(frames, vec![String::from("x"), String::from("y")]);
    assert_eq!(rig.events(), vec![String::from("wrap:requested")]);
}

#[test]
fn chunk_hook_sees_str_so_far() {
    let mut config = HookConfig::plain();
    config.per_chunk = true;
    let rig = Rig::new(config);
    let items = vec![
        Ok(model_chunk("Hel")),
        Ok(event_chunk(&[("content", Json::Text(String::from("lo")))])),
        Ok(text_chunk("!")),
    ];
    let frames = texts(rig.generator.sse_data_generator(call(items), None));
    assert_eq!(
        frames,
        vec![
            String::from("data: {\"choices\": [{\"delta\": {\"content\": \"Hel\"}}]}\n\n"),
            String::from("data: {\"content\": \"lo\"}\n\n"),
            String::from("!"),
        ]
    );
    assert_eq!(
        rig.events(),
        vec![
            String::from("wrap:requested"),
            String::from("chunk:"),
            String::from("chunk:Hel"),
            String::from("chunk:Hello"),
        ]
    );
}

#[test]
fn event_content_grows_text() {
    let mut config = HookConfig::plain();
    config.per_chunk = true;
    let rig = Rig::new(config);
    let items = vec![
        Ok(event_chunk(&[("content", Json::Null)])),
        Ok(event_chunk(&[("other", Json::Bool(true))])),
        Ok(event_chunk(&[(
            "content",
            Json::List(vec![Json::Bool(true)]),
        )])),
        Ok(event_chunk(&[("content", Json::Text(String::from("z")))])),
    ];
    let frames = texts(rig.generator.sse_data_generator(call(items), None));
    assert_eq!(frames.len(), 4);
    assert_eq!(
        rig.events(),
        vec![
            String::from("wrap:requested"),
            String::from("chunk:"),
            String::from("chunk:"),
            String::from("chunk:"),
            String::from("chunk:[true]"),
        ]
    );
    assert_eq!(frames[3], String::from("data: {\"content\": \"z\"}\n\n"));
}

#[test]
fn bytes_and_text_do_not_grow() {
    let mut config = HookConfig::plain();
    config.per_chunk = true;
    let rig = Rig::new(config);
    let items = vec![
        Ok(StreamItem::Bytes(Bytes::from_static(b"data: x\n\n"))),
        Ok(text_chunk("visible")),
    ];
    let frames = texts(rig.generator.sse_data_generator(call(items), None));
    assert_eq!(frames.len(), 2);
    assert_eq!(
        rig.events(),
        vec![
            String::from("wrap:requested"),
            String::from("chunk:"),
            String::from("chunk:"),
        ]
    );
}

#[test]
fn chunk_hook_replacement_is_written() {
    let mut config = HookConfig::plain();
    config.per_chunk = true;
    config.rewrite = true;
    let rig = Rig::new(config);
    let frames = texts(
        rig.generator
            .sse_data_generator(call(vec![Ok(text_chunk("x"))]), None),
    );
    assert_eq!(frames, vec![String::from("hooked")]);
}

#[test]
fn chunk_hook_failure_ends_stream() {
    let mut config = HookConfig::plain();
    config.per_chunk = true;
    config.chunk_failure = Some(status_failure("guardrail", 400));
    let rig = Rig::new(config);
    let results = drive(
        rig.generator
            .sse_data_generator(call(vec![Ok(text_chunk("x"))]), None),
    );
    assert_eq!(results.len(), 1);
    match results.first() {
        Some(Ok(Emitted::AnthropicError(frame))) => assert_eq!(frame.status_code, 400),
        Some(Ok(Emitted::Frame(_))) => panic!("expected the error frame"),
        Some(Err(_)) => panic!("expected the error frame"),
        None => panic!("no frame"),
    }
    assert_eq!(
        rig.events(),
        vec![
            String::from("wrap:requested"),
            String::from("chunk:"),
            String::from("failure:guardrail"),
        ]
    );
    let outcomes = rig.outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].stream_completed);
    assert!(!outcomes[0].client_disconnected);
}

#[test]
fn cost_injection_uses_model() {
    let rig = Rig::new(HookConfig::plain()).with_cost();
    let frames = texts(
        rig.generator
            .sse_data_generator(call(vec![Ok(text_chunk("x"))]), None),
    );
    assert_eq!(frames, vec![String::from("cost x")]);
    assert_eq!(rig.costs(), vec![String::from("requested")]);
}

#[test]
fn cost_without_a_text_model_injects_empty() {
    let rig = Rig::new(HookConfig::plain()).with_cost();
    let mut shared_call = call(vec![Ok(text_chunk("x"))]);
    shared_call.request_data = object(&[("model", Json::Null)]);
    let frames = texts(rig.generator.sse_data_generator(shared_call, None));
    assert_eq!(frames, vec![String::from("cost x")]);
    assert_eq!(rig.costs(), vec![String::new()]);
}

#[test]
fn cost_keeps_chunk_hook_on() {
    let rig = Rig::new(HookConfig::plain()).with_cost();
    let frames = texts(
        rig.generator
            .sse_data_generator(call(vec![Ok(text_chunk("x"))]), None),
    );
    assert_eq!(frames.len(), 1);
    assert_eq!(
        rig.events(),
        vec![String::from("wrap:requested"), String::from("chunk:")]
    );
}

#[test]
fn cleanup_after_clean_end() {
    let rig = Rig::new(HookConfig::plain());
    let frames = texts(
        rig.generator
            .sse_data_generator(call(vec![Ok(text_chunk("x")), Ok(text_chunk("y"))]), None),
    );
    assert_eq!(frames.len(), 2);
    let outcomes = rig.outcomes();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        outcomes[0],
        Outcome {
            key: String::from("key-1"),
            stream_completed: true,
            client_disconnected: false,
            snapshot: Some(String::from("snap")),
        }
    );
    assert_eq!(rig.releases().len(), 0);
}

#[test]
fn cleanup_once_after_failure() {
    let rig = Rig::new(HookConfig::plain());
    let results = drive(
        rig.generator
            .sse_data_generator(call(vec![Err(other_failure("boom"))]), None),
    );
    assert_eq!(results.len(), 1);
    let outcomes = rig.outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].stream_completed);
    assert!(!outcomes[0].client_disconnected);
}

struct AbortingFraming;

impl ChunkFraming for AbortingFraming {
    type Out = String;

    fn frame_chunk(&mut self, _item: StreamItem) -> Result<String, StreamAbort> {
        Err(StreamAbort {
            status: 502,
            detail: String::from("framing failed"),
        })
    }

    fn frame_tail(&mut self) -> Option<String> {
        None
    }

    fn frame_failure(&mut self, _error: &ProxyException) -> Result<String, StreamAbort> {
        Ok(String::from("failure"))
    }
}

#[test]
fn framing_failure_aborts_the_body() {
    let rig = Rig::new(HookConfig::plain());
    let results = drive(
        rig.generator
            .generate(call(vec![Ok(text_chunk("x"))]), AbortingFraming),
    );
    assert_eq!(results.len(), 1);
    let abort = abort_of(&results, 0);
    assert_eq!(abort.status, 502);
    assert_eq!(abort.detail, "framing failed");
    let outcomes = rig.outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(!outcomes[0].stream_completed);
    assert!(!outcomes[0].client_disconnected);
}

#[test]
fn drop_before_chunk_releases() {
    let rig = Rig::new(HookConfig::plain());
    let ping = StreamItem::Bytes(Bytes::from_static(ANTHROPIC_PING_BYTES));
    let mut body = rig.generator.sse_data_generator(call(vec![Ok(ping)]), None);
    let first = Runtime::new().expect("runtime").block_on(body.next());
    assert_eq!(
        text_of(first.expect("frame").expect("ok")),
        ANTHROPIC_PING_CHUNK
    );
    drop(body);
    assert_eq!(rig.releases(), vec![String::from("key-1")]);
    let outcomes = rig.outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(!outcomes[0].stream_completed);
    assert!(outcomes[0].client_disconnected);
}

#[test]
fn drop_after_chunk_keeps_charge() {
    let rig = Rig::new(HookConfig::plain());
    let mut body = rig
        .generator
        .sse_data_generator(call(vec![Ok(text_chunk("x"))]), None);
    let first = Runtime::new().expect("runtime").block_on(body.next());
    assert_eq!(text_of(first.expect("frame").expect("ok")), "x");
    drop(body);
    assert_eq!(rig.releases().len(), 0);
    let outcomes = rig.outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].client_disconnected);
}

#[test]
fn drop_at_the_error_frame_still_counts_the_disconnect() {
    let rig = Rig::new(HookConfig::plain());
    let mut body = rig
        .generator
        .sse_data_generator(call(vec![Err(other_failure("boom"))]), None);
    let first = Runtime::new().expect("runtime").block_on(body.next());
    match first.expect("frame") {
        Ok(Emitted::AnthropicError(frame)) => assert_eq!(frame.status_code, 500),
        Ok(Emitted::Frame(_)) => panic!("expected the typed error frame"),
        Err(_) => panic!("expected the error frame"),
    }
    drop(body);
    assert_eq!(rig.releases(), vec![String::from("key-1")]);
    let outcomes = rig.outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].stream_completed);
    assert!(!outcomes[0].client_disconnected);
}

#[test]
fn the_tail_is_written_before_the_stream_completes() {
    let rig = Rig::new(HookConfig::plain());
    let torn = StreamItem::Bytes(Bytes::from_static(b"data: {\"x\":"));
    let mut body = rig
        .generator
        .sse_data_generator(call(vec![Ok(torn)]), Some(String::from("client-model")));
    let runtime = Runtime::new().expect("runtime");
    let first = runtime.block_on(body.next());
    assert_eq!(text_of(first.expect("frame").expect("ok")), "");
    let second = runtime.block_on(body.next());
    assert_eq!(
        text_of(second.expect("frame").expect("ok")),
        "data: {\"x\":"
    );
    drop(body);
    let outcomes = rig.outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(!outcomes[0].stream_completed);
    assert!(outcomes[0].client_disconnected);
    assert_eq!(rig.releases().len(), 0);
}

#[test]
fn polling_past_the_tail_completes_the_cleanup() {
    let rig = Rig::new(HookConfig::plain());
    let torn = StreamItem::Bytes(Bytes::from_static(b"data: {\"x\":"));
    let mut body = rig
        .generator
        .sse_data_generator(call(vec![Ok(torn)]), Some(String::from("client-model")));
    let runtime = Runtime::new().expect("runtime");
    runtime.block_on(body.next()).expect("frame").expect("ok");
    runtime.block_on(body.next()).expect("frame").expect("ok");
    match runtime.block_on(body.next()) {
        None => {}
        Some(_) => panic!("the stream ended after the tail"),
    }
    let outcomes = rig.outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].stream_completed);
    assert!(!outcomes[0].client_disconnected);
}

#[test]
fn text_ping_counts_delivered() {
    let rig = Rig::new(HookConfig::plain());
    let ping_text = StreamItem::Text(ANTHROPIC_PING_CHUNK.to_owned());
    let mut body = rig
        .generator
        .sse_data_generator(call(vec![Ok(ping_text)]), None);
    let first = Runtime::new().expect("runtime").block_on(body.next());
    assert_eq!(
        text_of(first.expect("frame").expect("ok")),
        ANTHROPIC_PING_CHUNK
    );
    drop(body);
    assert_eq!(rig.releases().len(), 0);
}

#[test]
fn buffered_output_keeps_charge() {
    let rig = Rig::new(HookConfig::plain());
    let ping = StreamItem::Bytes(Bytes::from_static(ANTHROPIC_PING_BYTES));
    let mut shared_call = call(vec![Ok(ping)]);
    shared_call.source.buffered = Some(Arc::new(FixedBuffered(true)));
    let mut body = rig.generator.sse_data_generator(shared_call, None);
    let first = Runtime::new().expect("runtime").block_on(body.next());
    assert_eq!(
        text_of(first.expect("frame").expect("ok")),
        ANTHROPIC_PING_CHUNK
    );
    drop(body);
    assert_eq!(rig.releases().len(), 0);
}

#[test]
fn pending_stream_times_out_disconnected() {
    let rig = Rig::new(HookConfig::plain());
    let mut body = rig.generator.sse_data_generator(pending_call(), None);
    let polled = Runtime::new().expect("runtime").block_on(async {
        tokio::time::timeout(std::time::Duration::from_millis(20), body.next()).await
    });
    polled.expect_err("the stream stays pending");
    drop(body);
    assert_eq!(rig.releases(), vec![String::from("key-1")]);
    let outcomes = rig.outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].client_disconnected);
}

mod failure;
mod sse;
