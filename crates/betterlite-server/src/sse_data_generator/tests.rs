#![cfg(test)]

use super::*;
use async_trait::async_trait;
use futures_util::StreamExt as _;
use futures_util::stream;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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

#[test]
fn failure_frame_preserves_null_and_missing_fields() {
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
}

#[test]
fn failure_frame_applies_router_status_overrides() {
    let mut failure = StreamFailure {
        message: "No deployments available".to_owned(),
        error_type: Some("Unavailable".to_owned()),
        param: FailureParam::Missing,
        status_code: Some(500),
        kind: FailureKind::Other,
    };
    assert!(failure_frame(&failure).contains("\"code\":\"429\""));
    failure.message = "Not allowed to access model due to tags configuration".to_owned();
    assert!(failure_frame(&failure).contains("\"code\":\"401\""));
}

#[test]
fn unix_timestamp_uses_seconds_since_epoch() {
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_secs();
    let actual = unix_timestamp();
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_secs();
    let actual = u64::try_from(actual).expect("timestamp is nonnegative");
    assert!((before..=after).contains(&actual));
}

#[test]
fn cleanup_guard_finalizes_once_and_marks_incomplete_drop() {
    use std::sync::atomic::{AtomicBool, Ordering};

    struct RecordingCleanup(Arc<AtomicBool>);
    impl StreamCleanup<()> for RecordingCleanup {
        fn finalize(&self, outcome: CleanupOutcome<()>) {
            assert!(outcome.client_disconnected);
            assert!(!outcome.stream_completed);
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let called = Arc::new(AtomicBool::new(false));
    let guard = CleanupGuard {
        cleanup: Arc::new(RecordingCleanup(called.clone())),
        outcome: Some(CleanupOutcome {
            request: None,
            request_data: JsonObject::new(),
            key: (),
            response: None,
            stream_completed: false,
            client_disconnected: false,
        }),
    };
    drop(guard);
    assert!(called.load(Ordering::SeqCst));
}

#[test]
fn cleanup_guard_complete_marks_a_finished_stream_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingCleanup(Arc<AtomicUsize>);
    impl StreamCleanup<()> for RecordingCleanup {
        fn finalize(&self, outcome: CleanupOutcome<()>) {
            assert!(outcome.stream_completed);
            assert!(!outcome.client_disconnected);
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let mut guard = CleanupGuard {
        cleanup: Arc::new(RecordingCleanup(calls.clone())),
        outcome: Some(CleanupOutcome {
            request: None,
            request_data: JsonObject::new(),
            key: (),
            response: None,
            stream_completed: false,
            client_disconnected: false,
        }),
    };
    guard.complete(true);
    drop(guard);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn keepalive_source_exposes_item_hidden_params_and_skips_failures() {
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
    let source = StreamSource {
        items: stream::iter(items).boxed(),
        hidden: JsonObject::new(),
        snapshot: None,
    };
    let generator = SseDataGenerator::new(
        Arc::new(NoHooks),
        Arc::new(NoErrorState),
        Arc::new(NoCleanup),
        None,
        None,
        false,
        |_| false,
    );
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

#[test]
fn relays_raw_sse_frames_and_flushes_a_partial_frame() {
    let data = [("_litellm_raw_sse_stream".to_owned(), Json::Bool(true))]
        .into_iter()
        .collect();
    let frames = collect_frames(
        vec![
            Ok(StreamItem::Bytes(bytes::Bytes::from_static(
                b"data: one\n\n",
            ))),
            Ok(StreamItem::Text("data: two".to_owned())),
        ],
        data,
    );
    assert_eq!(frames, ["data: one\n\n", "data: two\n\n", DONE_FRAME]);
}

#[test]
fn relays_non_raw_sse_bytes_and_omits_done_when_requested() {
    let data = [(
        "_litellm_skip_openai_stream_done".to_owned(),
        Json::Bool(true),
    )]
    .into_iter()
    .collect();
    let frames = collect_frames(
        vec![Ok(StreamItem::Bytes(bytes::Bytes::from_static(
            b"event: update\n\n",
        )))],
        data,
    );
    assert_eq!(frames, ["event: update\n\n"]);
}

#[test]
fn text_error_message_is_emitted_before_done() {
    let frames = collect_frames(
        vec![Ok(StreamItem::Text("data: failed".to_owned()))],
        JsonObject::new(),
    );
    assert_eq!(frames, ["data: failed", DONE_FRAME]);
}

#[test]
fn plain_failure_writes_an_error_frame_without_done() {
    let failure = StreamFailure {
        message: "upstream failed".to_owned(),
        error_type: None,
        param: FailureParam::Missing,
        status_code: None,
        kind: FailureKind::Other,
    };
    let frames = collect_frames(vec![Err(failure)], JsonObject::new());
    assert_eq!(frames.len(), 1);
    assert!(frames[0].contains("upstream failed"));
    assert!(!frames[0].contains("[DONE]"));
}

#[tokio::test(start_paused = true)]
async fn quiet_stream_emits_a_keepalive_ping() {
    let generator = SseDataGenerator::new(
        Arc::new(NoHooks),
        Arc::new(NoErrorState),
        Arc::new(NoCleanup),
        None,
        Some(crate::keepalive_interval::PingSetting::Seconds(1.0)),
        false,
        |_| false,
    );
    let source = StreamSource {
        items: stream::pending().boxed(),
        hidden: JsonObject::new(),
        snapshot: None,
    };
    let mut body = generator.generate(GeneratorInput {
        response: source,
        key: (),
        request_data: JsonObject::new(),
        request: None,
        responses_stream_errors: false,
    });
    let ping = body.next().await.expect("ping").expect("body frame");
    assert_eq!(ping, bytes::Bytes::from_static(b": ping\n\n"));
}

#[test]
fn emits_data_chunks_then_done() {
    let chunk = StreamChunk {
        body: [
            ("id".to_owned(), Json::Text("1".to_owned())),
            ("model".to_owned(), Json::Text("wrong".to_owned())),
            ("choices".to_owned(), Json::List(Vec::new())),
        ]
        .into_iter()
        .collect(),
        kind: ChunkKind::ModelStream,
        hidden: JsonObject::new(),
    };
    let source = StreamSource {
        items: stream::iter([Ok(StreamItem::Chunk(chunk))]).boxed(),
        hidden: JsonObject::new(),
        snapshot: None,
    };
    let generator = SseDataGenerator::new(
        Arc::new(NoHooks),
        Arc::new(NoErrorState),
        Arc::new(NoCleanup),
        None,
        None,
        false,
        |_| false,
    );
    let body = generator.generate(GeneratorInput {
        response: source,
        key: (),
        request_data: [("model".to_owned(), Json::Text("requested".to_owned()))]
            .into_iter()
            .collect(),
        request: None,
        responses_stream_errors: false,
    });
    let result = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(body.collect::<Vec<_>>());
    let text = result
        .into_iter()
        .map(|frame| String::from_utf8(frame.expect("frame").to_vec()).expect("utf8"))
        .collect::<String>();
    assert!(text.contains("\"model\":\"requested\""));
    assert!(text.ends_with(DONE_FRAME));
}
