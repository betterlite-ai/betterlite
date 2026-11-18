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
