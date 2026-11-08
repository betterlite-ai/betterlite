#![cfg(test)]

use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::thread::{ThreadId, current};

use rstest::rstest;
use serde_json::{Map, Value};
use tokio::runtime::{Builder, Handle};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Metadata, Subscriber};

use super::{
    MirrorArgs, MirrorCall, MirrorInvocation, SilentExperimentError, SilentExperiments,
    silent_kwargs, silent_targets,
};

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn request(text: &str) -> Map<String, Value> {
    serde_json::from_str(text).unwrap()
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<String>>>);

impl Captured {
    fn text(&self) -> String {
        self.0.lock().unwrap().join("\n")
    }
}

impl Subscriber for Captured {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut message = MessageText::default();
        event.record(&mut message);
        self.0.lock().unwrap().push(message.text);
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[derive(Default)]
struct MessageText {
    text: String,
}

impl Visit for MessageText {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.text = format!("{value:?}");
        }
    }
}

fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let guard = set_default(captured.clone());
    (captured, guard)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Message(String);

#[derive(Debug, thiserror::Error)]
#[error("the fake mirror failed")]
struct MirrorFailure;

struct FakeMirror {
    calls: Arc<Mutex<Vec<MirrorArgs<Message>>>>,
    threads: Arc<Mutex<Vec<ThreadId>>>,
    fails: bool,
}

fn fake_mirror(fails: bool) -> Arc<FakeMirror> {
    Arc::new(FakeMirror {
        calls: Arc::new(Mutex::new(Vec::new())),
        threads: Arc::new(Mutex::new(Vec::new())),
        fails,
    })
}

impl FakeMirror {
    fn calls(&self) -> Vec<MirrorArgs<Message>> {
        self.calls.lock().unwrap().clone()
    }

    fn threads(&self) -> Vec<ThreadId> {
        self.threads.lock().unwrap().clone()
    }
}

impl MirrorCall<Message> for FakeMirror {
    type Error = MirrorFailure;

    fn complete(
        &self,
        args: MirrorArgs<Message>,
    ) -> Pin<Box<dyn Future<Output = Result<(), MirrorFailure>> + Send + 'static>> {
        self.calls.lock().unwrap().push(args);
        self.threads.lock().unwrap().push(current().id());
        let outcome = if self.fails {
            Err(MirrorFailure)
        } else {
            Ok(())
        };
        Box::pin(async move { outcome })
    }
}

#[rstest]
#[case::absent(None, Vec::new())]
#[case::null(Some(value("null")), Vec::new())]
#[case::one(Some(value("\"gpt-4o-mini\"")), vec!["gpt-4o-mini".to_owned()])]
#[case::many(
    Some(value("[\"gpt-4o-mini\", \"claude-3-haiku\"]")),
    vec!["gpt-4o-mini".to_owned(), "claude-3-haiku".to_owned()]
)]
#[case::empty_list(Some(value("[]")), Vec::new())]
fn targets_resolve_a_name_or_a_list(
    #[case] silent_model: Option<Value>,
    #[case] expected: Vec<String>,
) {
    assert_eq!(silent_targets(silent_model.as_ref()), expected);
}

#[rstest]
#[case::number(value("7"))]
#[case::boolean(value("true"))]
#[case::object(value("{\"name\": \"gpt-4o-mini\"}"))]
#[case::mixed_list(value("[\"gpt-4o-mini\", 3]"))]
fn targets_skip_an_unknown_shape_with_a_warning(#[case] silent_model: Value) {
    let (captured, _guard) = capture();
    assert!(silent_targets(Some(&silent_model)).is_empty());
    assert!(
        captured
            .text()
            .contains("silent_model must be a model name or a list of model names"),
        "{}",
        captured.text()
    );
}

#[rstest]
#[case::absent("{}")]
#[case::null("{\"metadata\": null}")]
fn kwargs_fill_missing_metadata(#[case] text: &str) {
    let silent = silent_kwargs(&request(text)).unwrap();
    assert_eq!(
        silent["metadata"],
        value("{\"is_silent_experiment\": true}")
    );
}

#[rstest]
#[case::text("{\"metadata\": \"nope\"}")]
#[case::number("{\"metadata\": 5}")]
#[case::array("{\"metadata\": []}")]
fn kwargs_reject_metadata_that_is_not_an_object(#[case] text: &str) {
    assert!(matches!(
        silent_kwargs(&request(text)),
        Err(SilentExperimentError::MetadataNotObject)
    ));
}

#[test]
fn kwargs_drop_the_parent_span_and_set_the_flag() {
    let original =
        request("{\"metadata\": {\"litellm_parent_otel_span\": \"span\", \"user\": \"alice\"}}");
    let silent = silent_kwargs(&original).unwrap();
    assert!(silent["metadata"].get("litellm_parent_otel_span").is_none());
    assert_eq!(silent["metadata"]["is_silent_experiment"], value("true"));
    assert_eq!(silent["metadata"]["user"], value("\"alice\""));
}

#[test]
fn kwargs_drop_logging_keys_and_keep_the_proxy_request() {
    let original = request(
        "{\"litellm_call_id\": \"id\", \"litellm_logging_obj\": {\"a\": 1}, \"standard_logging_object\": {\"b\": 2}, \"proxy_server_request\": {\"url\": \"http://proxy\"}, \"stream\": true}",
    );
    let silent = silent_kwargs(&original).unwrap();
    assert!(silent.get("litellm_call_id").is_none());
    assert!(silent.get("litellm_logging_obj").is_none());
    assert!(silent.get("standard_logging_object").is_none());
    assert_eq!(
        silent["proxy_server_request"],
        value("{\"url\": \"http://proxy\"}")
    );
    assert_eq!(silent["stream"], value("true"));
}

#[test]
fn kwargs_leave_the_caller_request_unchanged() {
    let original = request(
        "{\"metadata\": {\"litellm_parent_otel_span\": \"span\"}, \"litellm_call_id\": \"id\"}",
    );
    let before = original.clone();
    let silent = silent_kwargs(&original).unwrap();
    assert!(!silent.is_empty());
    assert_eq!(original, before);
}

fn experiments_on(handle: Handle) -> SilentExperiments {
    SilentExperiments { runtime: handle }
}

fn invocation<C>(
    call: Arc<C>,
    target: &str,
    messages: Vec<Message>,
    request: Map<String, Value>,
) -> MirrorInvocation<C, Message> {
    MirrorInvocation {
        call,
        target: target.to_owned(),
        messages,
        request,
    }
}

#[tokio::test]
async fn start_runs_the_mirror_with_the_target_and_the_stream_flag() {
    let experiments = experiments_on(Handle::current());
    let fake = fake_mirror(false);
    let original = request("{\"stream\": true, \"model\": \"gpt-4o\", \"metadata\": {}}");
    let caller_request = original.clone();
    let handle = experiments.start(invocation(
        Arc::clone(&fake),
        "gpt-4o-mini",
        vec![Message("hi".to_owned())],
        original,
    ));
    handle.await.unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    let args = &calls[0];
    assert_eq!(args.model, "gpt-4o-mini");
    assert_eq!(args.messages, vec![Message("hi".to_owned())]);
    assert!(args.stream);
    assert_eq!(
        args.request["metadata"]["model_group"],
        value("\"gpt-4o-mini\"")
    );
    assert_eq!(
        args.request["metadata"]["is_silent_experiment"],
        value("true")
    );
    assert!(args.request.get("stream").is_none());
    assert_eq!(args.request["model"], value("\"gpt-4o\""));
    assert_eq!(
        caller_request,
        request("{\"stream\": true, \"model\": \"gpt-4o\", \"metadata\": {}}")
    );
}

#[tokio::test]
async fn start_skips_a_mirror_marked_as_a_silent_experiment() {
    let experiments = experiments_on(Handle::current());
    let fake = fake_mirror(false);
    let original = request("{\"metadata\": {\"is_silent_experiment\": true}}");
    experiments
        .start(invocation(
            Arc::clone(&fake),
            "gpt-4o-mini",
            vec![Message("hi".to_owned())],
            original,
        ))
        .await
        .unwrap();
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn start_logs_a_failure_and_keeps_it_from_the_caller() {
    let (captured, _guard) = capture();
    let experiments = experiments_on(Handle::current());
    let fake = fake_mirror(true);
    experiments
        .start(invocation(
            Arc::clone(&fake),
            "gpt-4o-mini",
            Vec::new(),
            Map::new(),
        ))
        .await
        .unwrap();
    assert_eq!(fake.calls().len(), 1);
    assert!(
        captured
            .text()
            .contains("Silent experiment failed for model gpt-4o-mini"),
        "{}",
        captured.text()
    );
}

#[test]
fn blocking_runs_the_mirror_on_its_own_thread() {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();
    let experiments = experiments_on(runtime.handle().clone());
    let fake = fake_mirror(false);
    experiments
        .start_blocking(invocation(
            Arc::clone(&fake),
            "gpt-4o-mini",
            vec![Message("hi".to_owned())],
            Map::new(),
        ))
        .join()
        .unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "gpt-4o-mini");
    let threads = fake.threads();
    assert_ne!(threads[0], current().id());
}

#[test]
fn blocking_keeps_a_failing_mirror_from_the_caller() {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();
    let experiments = experiments_on(runtime.handle().clone());
    let fake = fake_mirror(true);
    experiments
        .start_blocking(invocation(
            Arc::clone(&fake),
            "gpt-4o-mini",
            Vec::new(),
            Map::new(),
        ))
        .join()
        .unwrap();
    assert_eq!(fake.calls().len(), 1);
}
