use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{
    FixedSource, Fixture, NativeCall, NativeCallKind, RecordingNative, SPAN_URL, TRACE_URL, call,
    capture_logs, millis, object, sample_kwargs, sample_response,
};
use crate::opik::delivery::record::SpanType;

#[derive(Clone, Copy, Debug)]
enum Callback {
    Sync,
    Async,
}

#[tokio::test]
async fn construction_caches_the_client_get_client_cached_returns() {
    let native = Arc::new(RecordingNative::new());
    let mut fixture = Fixture::new();
    fixture.source = FixedSource::with_client(Arc::clone(&native));

    let _first = fixture.build(None);
    assert_eq!(fixture.source.installed_calls(), 1);
    let _second = fixture.build(None);
    assert_eq!(fixture.source.installed_calls(), 2);
    assert!(native.calls().is_empty());
}

#[rstest]
#[case::sync(Callback::Sync)]
#[case::async_callback(Callback::Async)]
#[tokio::test]
async fn native_success_calls_trace_then_span_with_the_payload_fields(#[case] callback: Callback) {
    let native = Arc::new(RecordingNative::new());
    let mut fixture = Fixture::new();
    fixture.source = FixedSource::with_client(Arc::clone(&native));
    let logger = fixture.build(None);
    let kwargs = sample_kwargs();
    let response = sample_response();

    match callback {
        Callback::Sync => logger.log_success_event(call(&kwargs, &response)).await,
        Callback::Async => {
            logger
                .async_log_success_event(call(&kwargs, &response))
                .await;
        }
    }

    assert!(fixture.delivery.requests().is_empty());
    let calls = native.calls();
    assert_eq!(calls.len(), 2);
    let NativeCall::Trace(trace) = &calls[0] else {
        panic!("trace must be called before span")
    };
    let NativeCall::Span(span) = &calls[1] else {
        panic!("span must be called second")
    };
    assert_eq!(trace.name, "chat.completion");
    assert_eq!(trace.start_time, millis(1_000));
    assert_eq!(trace.end_time, millis(2_000));
    assert_eq!(
        trace.input,
        Some(serde_json::from_str::<Json>(r#"[{"role": "user", "content": "hi"}]"#).unwrap())
    );
    assert_eq!(
        trace.output,
        Some(serde_json::from_str::<Json>(r#"{"answer": "ok"}"#).unwrap())
    );
    assert_eq!(trace.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(trace.project_name, "Default Project");
    assert_eq!(trace.tags, [Json::Text("openai".to_owned())]);
    assert_eq!(
        trace.metadata["created_from"],
        Json::Text("litellm".to_owned())
    );
    assert_eq!(span.trace_id, trace.id);
    assert_eq!(span.parent_span_id, None);
    assert_eq!(span.name, "gpt-4o_chat.completion_7");
    assert_eq!(span.span_type, SpanType::Llm);
    assert_eq!(span.model, "gpt-4o");
    assert_eq!(span.start_time, millis(1_000));
    assert_eq!(span.end_time, millis(2_000));
    assert_eq!(
        span.usage["prompt_tokens"],
        serde_json::from_str::<Json>("3").unwrap()
    );
    assert_eq!(
        span.usage["total_tokens"],
        serde_json::from_str::<Json>("7").unwrap()
    );
    assert_eq!(span.provider.as_deref(), Some("openai"));
    assert_eq!(span.total_cost, Some(Decimal::new(25, 2)));
    assert_eq!(span.tags, [Json::Text("openai".to_owned())]);
    assert_eq!(span.project_name, "Default Project");
}

#[tokio::test]
async fn native_attaches_the_span_to_an_existing_trace() {
    let native = Arc::new(RecordingNative::new());
    let mut fixture = Fixture::new();
    fixture.source = FixedSource::with_client(Arc::clone(&native));
    let logger = fixture.build(None);
    let kwargs = object(
        r#"{
            "standard_logging_object": {
                "messages": [{"role": "user", "content": "hi"}],
                "response": {"answer": "ok"},
                "model": "gpt-4o",
                "metadata": {
                    "requester_metadata": {
                        "opik": {"current_span_data": {"trace_id": "trace-existing", "id": "parent-span"}}
                    }
                }
            }
        }"#,
    );
    let response = sample_response();

    logger.log_success_event(call(&kwargs, &response)).await;

    let calls = native.calls();
    assert_eq!(calls.len(), 1);
    let NativeCall::Span(span) = &calls[0] else {
        panic!("an existing trace needs no trace call")
    };
    assert_eq!(span.trace_id, "trace-existing");
    assert_eq!(span.parent_span_id.as_deref(), Some("parent-span"));
}

#[tokio::test]
async fn native_receives_null_input_and_output_when_the_logging_object_has_nulls() {
    let native = Arc::new(RecordingNative::new());
    let mut fixture = Fixture::new();
    fixture.source = FixedSource::with_client(Arc::clone(&native));
    let logger = fixture.build(None);
    let kwargs = object(
        r#"{
            "standard_logging_object": {
                "messages": null,
                "response": null,
                "model": "gpt-4o"
            }
        }"#,
    );
    let response = sample_response();

    logger.log_success_event(call(&kwargs, &response)).await;

    let calls = native.calls();
    assert_eq!(calls.len(), 2);
    let NativeCall::Trace(trace) = &calls[0] else {
        panic!("trace must be called before span")
    };
    assert_eq!(trace.input, None);
    assert_eq!(trace.output, None);
    let NativeCall::Span(span) = &calls[1] else {
        panic!("span must be called second")
    };
    assert_eq!(span.input, None);
    assert_eq!(span.output, None);
}

#[tokio::test]
async fn native_trace_failure_skips_the_span_and_logs() {
    let (logs, _guard) = capture_logs();
    let native = Arc::new(RecordingNative::failing(NativeCallKind::Trace));
    let mut fixture = Fixture::new();
    fixture.source = FixedSource::with_client(Arc::clone(&native));
    let logger = fixture.build(None);
    let kwargs = sample_kwargs();
    let response = sample_response();

    logger.log_success_event(call(&kwargs, &response)).await;

    let calls = native.calls();
    assert_eq!(calls.len(), 1);
    assert!(matches!(calls[0], NativeCall::Trace(_)));
    assert!(fixture.delivery.requests().is_empty());
    assert!(
        logs.contents()
            .contains("OpikLogger failed to log success event")
    );
}

#[rstest]
#[case::sync(Callback::Sync)]
#[case::async_callback(Callback::Async)]
#[tokio::test]
async fn native_span_failure_is_logged_without_propagating(#[case] callback: Callback) {
    let (logs, _guard) = capture_logs();
    let native = Arc::new(RecordingNative::failing(NativeCallKind::Span));
    let mut fixture = Fixture::new();
    fixture.source = FixedSource::with_client(Arc::clone(&native));
    let logger = fixture.build(None);
    let kwargs = sample_kwargs();
    let response = sample_response();

    match callback {
        Callback::Sync => logger.log_success_event(call(&kwargs, &response)).await,
        Callback::Async => {
            logger
                .async_log_success_event(call(&kwargs, &response))
                .await;
        }
    }

    assert_eq!(native.calls().len(), 2);
    assert!(fixture.delivery.requests().is_empty());
    assert!(
        logs.contents()
            .contains("OpikLogger failed to log success event")
    );
}

#[tokio::test]
async fn an_absent_native_client_falls_back_to_http() {
    let fixture = Fixture::new();
    let logger = fixture.build(None);
    assert_eq!(fixture.source.installed_calls(), 1);
    let kwargs = sample_kwargs();
    let response = sample_response();

    logger.log_success_event(call(&kwargs, &response)).await;

    assert_eq!(fixture.delivery.urls(), [TRACE_URL, SPAN_URL]);
}
