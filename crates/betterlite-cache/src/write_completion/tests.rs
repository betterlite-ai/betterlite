#![cfg(test)]

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_context::call_context::Phase;
use betterlite_context::local_ttl_cache::{CacheError, CacheValue};
use rstest::rstest;
use tokio::sync::Notify;

use crate::cache_key::value::KeyValue;
use crate::handler::call_types::call_type_name;
use crate::handler::value::to_cache_value;
use crate::types::cache_type::LiteLlmCacheType;
use crate::types::call_type::CachingSupportedCallType;
use crate::write_completion::store::{
    CacheContext, StoreRequest, WriteContext, store_response, store_response_sync,
};
use crate::write_completion::stream::{
    add_streaming_response_to_cache, add_sync_streaming_to_cache,
};
use crate::write_completion::tracker::{
    CACHE_WRITE_FLUSH_TIMEOUT, CacheWriteFuture, CacheWriteOutcome,
};
use crate::write_completion::wrap::{
    ResultKind, StreamCacheHandler, StreamCacheWrap, wrap_streaming_result_for_cache,
};
use support::{
    FakeAssembler, RecordedWrite, Recorder, SilentConsole, SilentLogger, Sink, cache,
    capture_warnings, captured, chat_json, chat_kwargs, chat_result, chunk, embedding_kwargs,
    embedding_response, embedding_result, handler, object, original, other_json, other_result,
    recording_dual, text, tracker, write_context,
};

#[tokio::test]
async fn tracker_runs_write_once_when_not_cancelled() {
    let tracker = tracker();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let handle = tracker.spawn(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        let future: CacheWriteFuture = Box::pin(async { Ok::<(), CacheError>(()) });
        future
    });
    let outcome = handle.task.await.expect("the write task ends");
    assert_eq!(outcome, CacheWriteOutcome::Completed);
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn tracker_reruns_write_after_cancel() {
    let tracker = tracker();
    let attempts = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(Notify::new());
    let counter = Arc::clone(&attempts);
    let signal = Arc::clone(&started);
    let handle = tracker.spawn(move || {
        let attempt = counter.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            signal.notify_one();
            let pending: CacheWriteFuture =
                Box::pin(std::future::pending::<Result<(), CacheError>>());
            return pending;
        }
        let done: CacheWriteFuture = Box::pin(async { Ok::<(), CacheError>(()) });
        done
    });
    started.notified().await;
    assert_eq!(attempts.load(Ordering::SeqCst), 1, "the write started");
    tracker.cancellation.cancel();
    let outcome = handle.task.await.expect("the write task ends");
    assert_eq!(outcome, CacheWriteOutcome::Cancelled);
    assert_eq!(attempts.load(Ordering::SeqCst), 2, "the factory runs again");
}

#[tokio::test]
async fn tracker_logs_failed_flush_and_ends_cancelled() {
    let (subscriber, bytes) = capture_warnings();
    let _guard = tracing::subscriber::set_default(subscriber);
    let tracker = tracker();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let handle = tracker.spawn(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        let future: CacheWriteFuture =
            Box::pin(async { Err::<(), CacheError>(CacheError::Unavailable) });
        future
    });
    tracker.cancellation.cancel();
    let outcome = handle.task.await.expect("the write task ends");
    assert_eq!(outcome, CacheWriteOutcome::Cancelled);
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let log = captured(&bytes);
    assert!(
        log.contains("LiteLLM Cache: pending cache write failed during event loop shutdown"),
        "{log}"
    );
}

#[tokio::test(start_paused = true)]
async fn tracker_flush_times_out_after_five_seconds() {
    let (subscriber, bytes) = capture_warnings();
    let _guard = tracing::subscriber::set_default(subscriber);
    let tracker = tracker();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let handle = tracker.spawn(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        let future: CacheWriteFuture = Box::pin(std::future::pending::<Result<(), CacheError>>());
        future
    });
    tracker.cancellation.cancel();
    let started = tokio::time::Instant::now();
    let outcome = handle.task.await.expect("the write task ends");
    assert_eq!(outcome, CacheWriteOutcome::Cancelled);
    assert_eq!(CACHE_WRITE_FLUSH_TIMEOUT, Duration::from_secs(5));
    assert!(
        started.elapsed() >= CACHE_WRITE_FLUSH_TIMEOUT,
        "the flush waits the full five seconds"
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let log = captured(&bytes);
    assert!(
        log.contains("pending cache write failed during event loop shutdown"),
        "{log}"
    );
}

#[tokio::test]
async fn tracker_forgets_finished_tasks() {
    let tracker = tracker();
    let handle = tracker.spawn(|| {
        let future: CacheWriteFuture = Box::pin(async { Ok::<(), CacheError>(()) });
        future
    });
    let _outcome = handle.task.await.expect("the write task ends");
    assert!(
        tracker.tasks.is_empty(),
        "the finished write leaves the tracker"
    );
    assert_eq!(tracker.tasks.len(), 0);
}

#[tokio::test]
async fn tracker_runs_write_in_post_response_phase() {
    let tracker = tracker();
    let handle = tracker.spawn(|| {
        let future: CacheWriteFuture = Box::pin(async { Ok::<(), CacheError>(()) });
        future
    });
    assert_eq!(handle.phase, Phase::PostResponse);
    let outcome = handle.task.await.expect("the write task ends");
    assert_eq!(outcome, CacheWriteOutcome::Completed);
}

#[tokio::test]
async fn store_skips_without_cache() {
    let recorder = Arc::new(Recorder::default());
    let context: WriteContext<SilentLogger, SilentConsole> = WriteContext {
        cache: CacheContext {
            cache: None,
            supported_call_types: Some(vec![CachingSupportedCallType::Acompletion]),
        },
        tracker: Arc::new(tracker()),
    };
    let dual = recording_dual(&recorder);
    let result = chat_result();
    let function = original("acompletion", &["model", "messages"]);
    let kwargs = chat_kwargs();
    store_response(
        &context,
        Some(&dual),
        &StoreRequest {
            result: &result,
            original_function: &function,
            kwargs: &kwargs,
            args: &[],
        },
    )
    .expect("a skipped write is not an error");
    assert!(recorder.writes().is_empty(), "no cache means no write");
    assert_eq!(context.tracker.tasks.len(), 0);
}

#[tokio::test]
async fn store_skips_unsupported_call_type() {
    let recorder = Arc::new(Recorder::default());
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
    );
    let dual = recording_dual(&recorder);
    let result = chat_result();
    let function = original("aembedding", &["model", "input"]);
    let kwargs = embedding_kwargs(&["text"]);
    store_response(
        &context,
        Some(&dual),
        &StoreRequest {
            result: &result,
            original_function: &function,
            kwargs: &kwargs,
            args: &[],
        },
    )
    .expect("a skipped write is not an error");
    assert!(
        recorder.writes().is_empty(),
        "an unlisted call type means no write"
    );
    assert_eq!(context.tracker.tasks.len(), 0);
}

#[tokio::test]
async fn store_skips_no_store_control() {
    let recorder = Arc::new(Recorder::default());
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
    );
    let dual = recording_dual(&recorder);
    let result = chat_result();
    let function = original("acompletion", &["model", "messages"]);
    let mut kwargs = chat_kwargs();
    kwargs.insert(
        "cache".to_owned(),
        object(&[("no-store", KeyValue::Bool(true))]),
    );
    store_response(
        &context,
        Some(&dual),
        &StoreRequest {
            result: &result,
            original_function: &function,
            kwargs: &kwargs,
            args: &[],
        },
    )
    .expect("a skipped write is not an error");
    assert!(
        recorder.writes().is_empty(),
        "the no-store control means no write"
    );
    assert_eq!(context.tracker.tasks.len(), 0);
}

#[tokio::test]
async fn store_merges_positional_args_over_kwargs() {
    let result = chat_result();
    let function = original("acompletion", &["model", "messages"]);

    let merged_recorder = Arc::new(Recorder::default());
    let merged_context = write_context(
        &merged_recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
    );
    let merged_dual = recording_dual(&merged_recorder);
    let original_kwargs = chat_kwargs();
    let args = [text("gpt-4o"), KeyValue::Array(Vec::new())];
    store_response(
        &merged_context,
        Some(&merged_dual),
        &StoreRequest {
            result: &result,
            original_function: &function,
            kwargs: &original_kwargs,
            args: &args,
        },
    )
    .expect("the merged write starts");
    merged_context.tracker.wait_idle().await;
    assert_eq!(
        original_kwargs.get("model"),
        Some(&text("gpt-4")),
        "the caller's kwargs stay unchanged"
    );

    let control_recorder = Arc::new(Recorder::default());
    let control_context = write_context(
        &control_recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
    );
    let control_dual = recording_dual(&control_recorder);
    let mut control_kwargs = chat_kwargs();
    control_kwargs.insert("model".to_owned(), text("gpt-4o"));
    store_response(
        &control_context,
        Some(&control_dual),
        &StoreRequest {
            result: &result,
            original_function: &function,
            kwargs: &control_kwargs,
            args: &[],
        },
    )
    .expect("the control write starts");
    control_context.tracker.wait_idle().await;

    let overridden_recorder = Arc::new(Recorder::default());
    let overridden_context = write_context(
        &overridden_recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
    );
    let overridden_dual = recording_dual(&overridden_recorder);
    let overridden_kwargs = chat_kwargs();
    store_response(
        &overridden_context,
        Some(&overridden_dual),
        &StoreRequest {
            result: &result,
            original_function: &function,
            kwargs: &overridden_kwargs,
            args: &[],
        },
    )
    .expect("the overridden write starts");
    overridden_context.tracker.wait_idle().await;

    let merged = single_write(&merged_recorder);
    let control = single_write(&control_recorder);
    let overridden = single_write(&overridden_recorder);
    assert_eq!(
        merged.key, control.key,
        "positional arguments replace the kwargs of the same name"
    );
    assert_ne!(
        merged.key, overridden.key,
        "the positional model changes the derived key"
    );
}

#[tokio::test]
async fn store_embedding_uses_pipeline() {
    let recorder = Arc::new(Recorder::default());
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Aembedding]),
    );
    let dual = recording_dual(&recorder);
    let result = embedding_result(2);
    let function = original("aembedding", &["model", "input"]);
    let kwargs = embedding_kwargs(&["first", "second"]);
    store_response(
        &context,
        Some(&dual),
        &StoreRequest {
            result: &result,
            original_function: &function,
            kwargs: &kwargs,
            args: &[],
        },
    )
    .expect("the pipeline write starts");
    context.tracker.wait_idle().await;
    let writes = recorder.writes();
    assert_eq!(writes.len(), 2, "one entry per embedding input");
    let sinks: Vec<Sink> = writes.iter().map(|write| write.sink).collect();
    assert_eq!(
        sinks,
        vec![Sink::Dual, Sink::Dual],
        "the pipeline writes through the dual cache"
    );
}

#[tokio::test]
async fn store_embedding_on_s3_uses_single_write() {
    let recorder = Arc::new(Recorder::default());
    let context = write_context(
        &recorder,
        LiteLlmCacheType::S3,
        Some(vec![CachingSupportedCallType::Aembedding]),
    );
    let dual = recording_dual(&recorder);
    let result = embedding_result(2);
    let function = original("aembedding", &["model", "input"]);
    let kwargs = embedding_kwargs(&["first", "second"]);
    store_response(
        &context,
        Some(&dual),
        &StoreRequest {
            result: &result,
            original_function: &function,
            kwargs: &kwargs,
            args: &[],
        },
    )
    .expect("the single write starts");
    context.tracker.wait_idle().await;
    let write = single_write(&recorder);
    assert_eq!(write.sink, Sink::Dual);
    let expected =
        serde_json::to_string(&embedding_response(2)).expect("the embedding fixture serializes");
    assert_eq!(write.response, CacheValue::Text(expected));
}

#[tokio::test]
async fn store_typed_result_writes_json_string_with_dual_cache() {
    let recorder = Arc::new(Recorder::default());
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
    );
    let dual = recording_dual(&recorder);
    let result = chat_result();
    let function = original("acompletion", &["model", "messages"]);
    let kwargs = chat_kwargs();
    store_response(
        &context,
        Some(&dual),
        &StoreRequest {
            result: &result,
            original_function: &function,
            kwargs: &kwargs,
            args: &[],
        },
    )
    .expect("the typed write starts");
    context.tracker.wait_idle().await;
    let write = single_write(&recorder);
    assert_eq!(write.sink, Sink::Dual);
    let expected = serde_json::to_string(&chat_json()).expect("the chat fixture serializes");
    assert_eq!(write.response, CacheValue::Text(expected));
}

#[tokio::test]
async fn store_other_result_writes_raw_without_dual_cache() {
    let recorder = Arc::new(Recorder::default());
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
    );
    let dual = recording_dual(&recorder);
    let result = other_result();
    let function = original("acompletion", &["model", "messages"]);
    let kwargs = chat_kwargs();
    store_response(
        &context,
        Some(&dual),
        &StoreRequest {
            result: &result,
            original_function: &function,
            kwargs: &kwargs,
            args: &[],
        },
    )
    .expect("the raw write starts");
    context.tracker.wait_idle().await;
    let write = single_write(&recorder);
    assert_eq!(
        write.sink,
        Sink::Direct,
        "the raw write skips the dual cache"
    );
    let expected = to_cache_value(&other_json()).expect("the raw fixture converts");
    assert_eq!(write.response, expected);
}

#[test]
fn sync_store_writes_directly() {
    let recorder = Arc::new(Recorder::default());
    let context: CacheContext<SilentLogger, SilentConsole> = CacheContext {
        cache: Some(cache(&recorder, LiteLlmCacheType::Local)),
        supported_call_types: Some(vec![CachingSupportedCallType::Acompletion]),
    };
    let result = chat_result();
    let function = original("acompletion", &["model", "messages"]);
    let kwargs = chat_kwargs();
    store_response_sync(
        &context,
        &StoreRequest {
            result: &result,
            original_function: &function,
            kwargs: &kwargs,
            args: &[],
        },
    )
    .expect("the direct write succeeds");
    let write = single_write(&recorder);
    assert_eq!(write.sink, Sink::Direct);
    let expected = to_cache_value(&chat_json()).expect("the chat fixture converts");
    assert_eq!(
        write.response, expected,
        "the sync write stores the typed object"
    );
}

#[tokio::test]
async fn stream_chunks_wait_for_final_chunk() {
    let recorder = Arc::new(Recorder::default());
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
    );
    let mut handler = handler(chat_kwargs());
    let assembler = FakeAssembler {
        response: Mutex::new(Some(chat_json())),
        failure: false,
        calls: Mutex::new(Vec::new()),
    };
    add_streaming_response_to_cache(
        &chunk("hel", None),
        &mut handler,
        &assembler,
        1.25,
        &context,
    )
    .expect("a chunk without a finish reason is only buffered");
    assert!(
        recorder.writes().is_empty(),
        "no write before the final chunk"
    );
    assert!(
        assembler.calls().is_empty(),
        "no assembly before the final chunk"
    );
    add_streaming_response_to_cache(
        &chunk("", Some("stop")),
        &mut handler,
        &assembler,
        2.5,
        &context,
    )
    .expect("the final chunk is written");
    context.tracker.wait_idle().await;
    let write = single_write(&recorder);
    assert_eq!(write.sink, Sink::Direct);
    let expected = serde_json::to_string(&chat_json()).expect("the chat fixture serializes");
    assert_eq!(write.response, CacheValue::Text(expected));
    let calls = assembler.calls();
    assert_eq!(calls.len(), 1, "the final chunk assembles once");
    assert_eq!(calls[0].chunks.len(), 2, "both chunks reach the assembler");
    assert!(calls[0].is_async);
    assert_eq!(calls[0].start_time.to_bits(), 0.0_f64.to_bits());
    assert_eq!(calls[0].end_time.to_bits(), 2.5_f64.to_bits());
    assert_eq!(handler.async_streaming_chunks.len(), 2);
}

#[tokio::test]
async fn stream_writes_assembled_response_once() {
    let recorder = Arc::new(Recorder::default());
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
    );
    let mut handler = handler(chat_kwargs());
    let assembler = FakeAssembler {
        response: Mutex::new(Some(chat_json())),
        failure: false,
        calls: Mutex::new(Vec::new()),
    };
    add_streaming_response_to_cache(&chunk("one", None), &mut handler, &assembler, 1.0, &context)
        .expect("a chunk without a finish reason is only buffered");
    add_streaming_response_to_cache(&chunk("two", None), &mut handler, &assembler, 2.0, &context)
        .expect("a chunk without a finish reason is only buffered");
    add_streaming_response_to_cache(
        &chunk("three", Some("stop")),
        &mut handler,
        &assembler,
        3.0,
        &context,
    )
    .expect("the final chunk is written");
    context.tracker.wait_idle().await;
    let writes = recorder.writes();
    assert_eq!(writes.len(), 1, "the whole stream is stored once");
    let expected = serde_json::to_string(&chat_json()).expect("the chat fixture serializes");
    assert_eq!(writes[0].response, CacheValue::Text(expected));
    assert_eq!(assembler.calls().len(), 1);
    assert_eq!(handler.async_streaming_chunks.len(), 3);
}

#[rstest]
#[case::assembler_builds_nothing(false)]
#[case::assembler_fails(true)]
fn stream_without_assembly_writes_nothing(#[case] failure: bool) {
    let recorder = Arc::new(Recorder::default());
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
    );
    let mut handler = handler(chat_kwargs());
    let assembler = FakeAssembler {
        response: Mutex::new(None),
        failure,
        calls: Mutex::new(Vec::new()),
    };
    add_streaming_response_to_cache(
        &chunk("done", Some("stop")),
        &mut handler,
        &assembler,
        1.0,
        &context,
    )
    .expect("an unassembled chunk is not an error");
    assert!(
        recorder.writes().is_empty(),
        "only an assembled response is stored"
    );
    assert_eq!(context.tracker.tasks.len(), 0);
}

#[test]
fn sync_stream_writes_assembled_response_once() {
    let recorder = Arc::new(Recorder::default());
    let context: CacheContext<SilentLogger, SilentConsole> = CacheContext {
        cache: Some(cache(&recorder, LiteLlmCacheType::Local)),
        supported_call_types: Some(vec![CachingSupportedCallType::Acompletion]),
    };
    let mut handler = handler(chat_kwargs());
    let assembler = FakeAssembler {
        response: Mutex::new(Some(chat_json())),
        failure: false,
        calls: Mutex::new(Vec::new()),
    };
    add_sync_streaming_to_cache(
        &chunk("done", Some("stop")),
        &mut handler,
        &assembler,
        4.0,
        &context,
    )
    .expect("the final sync chunk is written");
    let write = single_write(&recorder);
    assert_eq!(write.sink, Sink::Direct);
    let expected = to_cache_value(&chat_json()).expect("the chat fixture converts");
    assert_eq!(
        write.response, expected,
        "the sync stream stores the typed object"
    );
    assert_eq!(handler.sync_streaming_chunks.len(), 1);
    assert!(handler.async_streaming_chunks.is_empty());
    let calls = assembler.calls();
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].is_async);
    assert_eq!(calls[0].end_time.to_bits(), 4.0_f64.to_bits());
}

#[rstest]
#[case::anthropic_messages_async_stream(
    CachingSupportedCallType::AnthropicMessages,
    true,
    Some(vec![CachingSupportedCallType::AnthropicMessages]),
    ResultKind::AsyncStream,
    StreamCacheWrap::Wrap
)]
#[case::aanthropic_messages_async_stream(
    CachingSupportedCallType::AanthropicMessages,
    true,
    Some(vec![CachingSupportedCallType::AanthropicMessages]),
    ResultKind::AsyncStream,
    StreamCacheWrap::Wrap
)]
#[case::other_call_type_passes_through(
    CachingSupportedCallType::Acompletion,
    true,
    Some(vec![CachingSupportedCallType::Acompletion]),
    ResultKind::AsyncStream,
    StreamCacheWrap::PassThrough
)]
#[case::missing_cache_passes_through(
    CachingSupportedCallType::AnthropicMessages,
    false,
    Some(vec![CachingSupportedCallType::AnthropicMessages]),
    ResultKind::AsyncStream,
    StreamCacheWrap::PassThrough
)]
#[case::missing_supported_types_passes_through(
    CachingSupportedCallType::AnthropicMessages,
    true,
    None,
    ResultKind::AsyncStream,
    StreamCacheWrap::PassThrough
)]
#[case::unsupported_call_type_passes_through(
    CachingSupportedCallType::AnthropicMessages,
    true,
    Some(vec![CachingSupportedCallType::Acompletion]),
    ResultKind::AsyncStream,
    StreamCacheWrap::PassThrough
)]
#[case::sync_stream_passes_through(
    CachingSupportedCallType::AnthropicMessages,
    true,
    Some(vec![CachingSupportedCallType::AnthropicMessages]),
    ResultKind::SyncStream,
    StreamCacheWrap::PassThrough
)]
#[case::finished_response_passes_through(
    CachingSupportedCallType::AnthropicMessages,
    true,
    Some(vec![CachingSupportedCallType::AnthropicMessages]),
    ResultKind::Response,
    StreamCacheWrap::PassThrough
)]
fn stream_wrap_decision_cases(
    #[case] call_type: CachingSupportedCallType,
    #[case] cache_configured: bool,
    #[case] supported: Option<Vec<CachingSupportedCallType>>,
    #[case] result_kind: ResultKind,
    #[case] expected: StreamCacheWrap,
) {
    let function = original(call_type_name(call_type), &["model", "messages"]);
    let kwargs = chat_kwargs();
    let handler = StreamCacheHandler {
        cache_configured,
        supported_call_types: supported.as_deref(),
        original_function: &function,
        request_kwargs: &kwargs,
    };
    let decision = wrap_streaming_result_for_cache(result_kind, call_type, &handler)
        .expect("the decision reads the request");
    assert_eq!(decision, expected);
}

fn single_write(recorder: &Recorder) -> RecordedWrite {
    let writes = recorder.writes();
    assert_eq!(writes.len(), 1, "expected exactly one cache write");
    writes
        .into_iter()
        .next()
        .expect("the single write is present")
}
