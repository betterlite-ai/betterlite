use opentelemetry::Context;
use opentelemetry::trace::{Span as _, Status};

use crate::support::{END, Rig, START, attach, call, in_memory_config, instant, simple_kwargs};

#[test]
fn legacy_lifecycle_success_without_a_parent_creates_the_primary_and_raw_spans() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let kwargs = simple_kwargs("call-1");
    lifecycle
        .handle_success(&call(&kwargs))
        .expect("success lifecycle");
    assert_eq!(rig.names(), vec!["litellm_request", "raw_gen_ai_request"]);
    let primary = rig.span("litellm_request");
    assert_eq!(primary.status, Status::Ok);
    assert_eq!(primary.start_time, instant(START));
    assert_eq!(primary.end_time, instant(END));
    let raw = rig.span("raw_gen_ai_request");
    assert_eq!(raw.parent_span_id, primary.span_context.span_id());
    assert_eq!(raw.status, Status::Ok);
    assert_eq!(raw.start_time, instant(START));
    assert_eq!(raw.end_time, instant(END));
}

#[test]
fn legacy_lifecycle_success_with_an_ambient_parent_keeps_the_hierarchy_shallow() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let parent = rig.parent(&lifecycle, "upstream");
    let kwargs = simple_kwargs("call-2");
    let context = attach(&parent.context);
    let guard = Context::attach(&context);
    lifecycle
        .handle_success(&call(&kwargs))
        .expect("success lifecycle");
    drop(guard);
    assert_eq!(rig.names(), vec!["raw_gen_ai_request"]);
    assert_eq!(
        rig.span("raw_gen_ai_request").parent_span_id,
        parent.context.span_id()
    );
    let data = parent
        .lock()
        .exported_data()
        .expect("the parent stays open");
    assert_eq!(data.status, Status::Ok);
    assert!(
        !data.attributes.is_empty(),
        "the shallow path stamps the parent span"
    );
}

#[test]
fn legacy_lifecycle_creates_the_primary_span_when_the_request_span_flag_is_on() {
    let rig = Rig::new();
    let lifecycle = rig.with_options(in_memory_config(), None, None, true);
    let parent = rig.parent(&lifecycle, "upstream");
    let kwargs = simple_kwargs("call-3");
    let context = attach(&parent.context);
    let guard = Context::attach(&context);
    lifecycle
        .handle_success(&call(&kwargs))
        .expect("success lifecycle");
    drop(guard);
    assert_eq!(rig.names(), vec!["litellm_request", "raw_gen_ai_request"]);
    let primary = rig.span("litellm_request");
    assert_eq!(primary.parent_span_id, parent.context.span_id());
}

#[test]
fn legacy_lifecycle_failure_without_a_parent_creates_the_error_span() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let kwargs = simple_kwargs("call-4");
    lifecycle
        .handle_failure(&call(&kwargs))
        .expect("failure lifecycle");
    let spans = rig.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, "litellm_request");
    assert!(matches!(spans[0].status, Status::Error { .. }));
    assert_eq!(spans[0].end_time, instant(END));
}

#[test]
fn legacy_lifecycle_failure_with_an_ambient_parent_stamps_the_parent() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let parent = rig.parent(&lifecycle, "upstream");
    let kwargs = simple_kwargs("call-5");
    let context = attach(&parent.context);
    let guard = Context::attach(&context);
    lifecycle
        .handle_failure(&call(&kwargs))
        .expect("failure lifecycle");
    drop(guard);
    assert!(rig.spans().is_empty(), "no new span is created");
    let data = parent
        .lock()
        .exported_data()
        .expect("the parent stays open");
    assert!(matches!(data.status, Status::Error { .. }));
    assert!(!data.attributes.is_empty());
}

#[test]
fn legacy_lifecycle_success_events_dedupe_the_sync_and_async_fire() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let kwargs = simple_kwargs("call-6");
    lifecycle
        .handle_success(&call(&kwargs))
        .expect("first success");
    lifecycle
        .handle_success(&call(&kwargs))
        .expect("duplicate success");
    assert_eq!(rig.names(), vec!["litellm_request", "raw_gen_ai_request"]);
}

#[test]
fn legacy_lifecycle_failure_events_dedupe_the_sync_and_async_fire() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let kwargs = simple_kwargs("call-7");
    lifecycle
        .handle_failure(&call(&kwargs))
        .expect("first failure");
    lifecycle
        .handle_failure(&call(&kwargs))
        .expect("duplicate failure");
    assert_eq!(rig.names(), vec!["litellm_request"]);
}
