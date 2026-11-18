use super::*;
use async_trait::async_trait;
use futures_util::stream;
use std::sync::Arc;

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
fn request_flag_requires_json_true() {
    assert!(flag(
        &[("enabled".to_owned(), Json::Bool(true)).into()],
        "enabled"
    ));
    assert!(!flag(
        &[("enabled".to_owned(), Json::Text("true".to_owned())).into()],
        "enabled"
    ));
    assert!(!flag(&JsonObject::new(), "enabled"));
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
