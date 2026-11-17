#![cfg(test)]

mod support;

use std::future::pending;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_context::call_context::{CallState, Phase};
use betterlite_context::local_ttl_cache::{CacheError, CacheValue};
use rstest::rstest;
use tokio::sync::Notify;
use tokio::time::Instant;
use tracing::subscriber::set_default;

use crate::backend::CacheBackend;
use crate::cache_key::value::{KeyObject, KeyValue};
use crate::dual_cache::DualCache;
use crate::handler::call_types::call_type_name;
use crate::handler::state::OriginalFunction;
use crate::handler::value::to_cache_value;
use crate::types::cache_type::LiteLlmCacheType;
use crate::types::call_type::CachingSupportedCallType;
use crate::write_completion::dual::SharedDualCache;
use crate::write_completion::request::add_parent_otel_span;
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
    FakeAssembler, RecordedWrite, Recorder, SilentConsole, SilentLogger, Sink, cache, call,
    capture_warnings, captured, chat_json, chat_kwargs, chat_result, chunk, dual_settings,
    embedding_kwargs, embedding_response, embedding_result, handler, memory, object, original,
    other_json, other_result, recording_dual, text, text_completion_json, tracker, write_context,
};

#[tokio::test]
async fn tracker_runs_write_once_when_not_cancelled() {
    let tracker = tracker();
    let call = call();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let handle = tracker.spawn(&call, move |_call: CallState| -> CacheWriteFuture {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok::<(), CacheError>(()) })
    });
    let outcome = handle.task.await.expect("the write task ends");
    assert_eq!(outcome, CacheWriteOutcome::Completed);
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn tracker_reruns_write_after_cancel() {
    let tracker = tracker();
    let call = call();
    let attempts = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(Notify::new());
    let counter = Arc::clone(&attempts);
    let signal = Arc::clone(&started);
    let handle = tracker.spawn(&call, move |_call: CallState| -> CacheWriteFuture {
        let attempt = counter.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            signal.notify_one();
            return Box::pin(pending::<Result<(), CacheError>>());
        }
        Box::pin(async { Ok::<(), CacheError>(()) })
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
    let _guard = set_default(subscriber);
    let tracker = tracker();
    let call = call();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let handle = tracker.spawn(&call, move |_call: CallState| -> CacheWriteFuture {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err::<(), CacheError>(CacheError::Unavailable) })
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
    let _guard = set_default(subscriber);
    let tracker = tracker();
    let call = call();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let handle = tracker.spawn(&call, move |_call: CallState| -> CacheWriteFuture {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::pin(pending::<Result<(), CacheError>>())
    });
    tracker.cancellation.cancel();
    let started = Instant::now();
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
    let call = call();
    let handle = tracker.spawn(&call, |_call: CallState| -> CacheWriteFuture {
        Box::pin(async { Ok::<(), CacheError>(()) })
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
    let call = call();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&observed);
    let handle = tracker.spawn(&call, move |state: CallState| -> CacheWriteFuture {
        let reported = Arc::clone(&seen);
        Box::pin(async move {
            reported
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(state.phase);
            Ok::<(), CacheError>(())
        })
    });
    let outcome = handle.task.await.expect("the write task ends");
    assert_eq!(outcome, CacheWriteOutcome::Completed);
    assert_eq!(
        *observed.lock().unwrap_or_else(PoisonError::into_inner),
        vec![Phase::PostResponse],
        "the write runs in the post-response phase"
    );
}

#[tokio::test]
async fn store_skips_without_cache() {
    let recorder = Arc::new(Recorder::default());
    let call = call();
    let context: WriteContext<'_, SilentLogger, SilentConsole> = WriteContext {
        cache: CacheContext {
            cache: None,
            supported_call_types: Some(vec![CachingSupportedCallType::Acompletion]),
        },
        tracker: Arc::new(tracker()),
        call: &call,
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
    let call = call();
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
        &call,
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
    let call = call();
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
        &call,
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
    let function = original("acompletion", &["model", "messages"]);
    let original_kwargs = chat_kwargs();
    let args = [text("gpt-4o"), KeyValue::Array(Vec::new())];
    let merged = stored_key(&function, &original_kwargs, &args).await;
    assert_eq!(
        original_kwargs.get("model"),
        Some(&text("gpt-4")),
        "the caller's kwargs stay unchanged"
    );
    let mut explicit_kwargs = chat_kwargs();
    explicit_kwargs.insert("model".to_owned(), text("gpt-4o"));
    let explicit = stored_key(&function, &explicit_kwargs, &[]).await;
    let unchanged = stored_key(&function, &chat_kwargs(), &[]).await;
    assert_eq!(
        merged, explicit,
        "positional arguments replace the kwargs of the same name"
    );
    assert_ne!(
        merged, unchanged,
        "the positional model changes the derived key"
    );
}

#[tokio::test]
async fn store_embedding_uses_pipeline() {
    let recorder = Arc::new(Recorder::default());
    let call = call();
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Aembedding]),
        &call,
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
    let call = call();
    let context = write_context(
        &recorder,
        LiteLlmCacheType::S3,
        Some(vec![CachingSupportedCallType::Aembedding]),
        &call,
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
    let call = call();
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
        &call,
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
    let call = call();
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
        &call,
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
fn store_resolves_the_parent_span_from_the_request() {
    let parent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    let stored = Some(KeyValue::String(parent.to_owned()));
    let mut plain = chat_kwargs();
    add_parent_otel_span(&mut plain);
    assert_eq!(plain.get("parent_otel_span"), Some(&KeyValue::Null));
    let mut metadata = chat_kwargs();
    metadata.insert(
        "metadata".to_owned(),
        object(&[("litellm_parent_otel_span", text(parent))]),
    );
    add_parent_otel_span(&mut metadata);
    assert_eq!(
        metadata.get("parent_otel_span"),
        stored.as_ref(),
        "metadata carries the parent span"
    );
    let mut params = chat_kwargs();
    params.insert(
        "litellm_params".to_owned(),
        object(&[(
            "metadata",
            object(&[("litellm_parent_otel_span", text(parent))]),
        )]),
    );
    add_parent_otel_span(&mut params);
    assert_eq!(
        params.get("parent_otel_span"),
        stored.as_ref(),
        "litellm_params metadata carries the parent span"
    );
    let mut direct = chat_kwargs();
    direct.insert("litellm_parent_otel_span".to_owned(), text(parent));
    add_parent_otel_span(&mut direct);
    assert_eq!(
        direct.get("parent_otel_span"),
        stored.as_ref(),
        "the span kwarg itself carries the parent span"
    );
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
    let expected = serde_json::to_string(&chat_json()).expect("the chat fixture serializes");
    assert_eq!(
        write.response,
        CacheValue::Text(expected),
        "the sync write stores the JSON string"
    );
}

#[tokio::test]
async fn stream_chunks_wait_for_final_chunk() {
    let recorder = Arc::new(Recorder::default());
    let call = call();
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
        &call,
    );
    let mut handler = handler(chat_kwargs());
    let assembler = FakeAssembler {
        response: Mutex::new(Some(chat_json())),
        failure: false,
        calls: Mutex::new(Vec::new()),
    };
    add_streaming_response_to_cache(
        &chunk("hello", None),
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
    assert_eq!(
        handler.async_streaming_chunks.len(),
        1,
        "the chunk waits in the buffer"
    );
}

#[tokio::test]
async fn stream_assembly_receives_the_call_times() {
    let recorder = Arc::new(Recorder::default());
    let call = call();
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
        &call,
    );
    let mut handler = handler(chat_kwargs());
    let assembler = FakeAssembler {
        response: Mutex::new(Some(chat_json())),
        failure: false,
        calls: Mutex::new(Vec::new()),
    };
    add_streaming_response_to_cache(
        &chunk("done", Some("stop")),
        &mut handler,
        &assembler,
        2.5,
        &context,
    )
    .expect("the final chunk assembles the stream");
    let calls = assembler.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].is_async);
    assert_eq!(calls[0].start_time.to_bits(), 0.0_f64.to_bits());
    assert_eq!(calls[0].end_time.to_bits(), 2.5_f64.to_bits());
}

#[tokio::test]
async fn stream_writes_assembled_response_once() {
    let recorder = Arc::new(Recorder::default());
    let call = call();
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
        &call,
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

#[tokio::test]
async fn stream_writes_text_completion_without_dual_cache() {
    let recorder = Arc::new(Recorder::default());
    let call = call();
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::AtextCompletion]),
        &call,
    );
    let mut handler = handler(chat_kwargs());
    handler.original_function = original("atext_completion", &["model", "prompt"]);
    handler.dual_cache = Some(DualCache::new(memory(), None, dual_settings()));
    let assembler = FakeAssembler {
        response: Mutex::new(Some(text_completion_json())),
        failure: false,
        calls: Mutex::new(Vec::new()),
    };
    add_streaming_response_to_cache(
        &chunk("", Some("stop")),
        &mut handler,
        &assembler,
        1.0,
        &context,
    )
    .expect("the final text completion chunk is written");
    context.tracker.wait_idle().await;
    let write = single_write(&recorder);
    assert_eq!(
        write.sink,
        Sink::Direct,
        "a text completion skips the handler's dual cache"
    );
    let expected = serde_json::to_string(&text_completion_json())
        .expect("the text completion fixture serializes");
    assert_eq!(write.response, CacheValue::Text(expected));
}

#[rstest]
#[case::assembler_builds_nothing(false)]
#[case::assembler_fails(true)]
fn stream_without_assembly_writes_nothing(#[case] failure: bool) {
    let recorder = Arc::new(Recorder::default());
    let call = call();
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
        &call,
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
    let expected = serde_json::to_string(&chat_json()).expect("the chat fixture serializes");
    assert_eq!(
        write.response,
        CacheValue::Text(expected),
        "the sync stream stores the JSON string"
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

#[test]
fn shared_dual_cache_uses_the_memory_default_ttl() {
    let recorder = Arc::new(Recorder::default());
    let dual = recording_dual(&recorder);
    let shared = SharedDualCache::shared(&dual);
    assert_eq!(
        shared.default_ttl(),
        600,
        "the shared tier keeps the memory cache's default ttl"
    );
}

async fn stored_key(function: &OriginalFunction, kwargs: &KeyObject, args: &[KeyValue]) -> String {
    let recorder = Arc::new(Recorder::default());
    let call = call();
    let context = write_context(
        &recorder,
        LiteLlmCacheType::Local,
        Some(vec![CachingSupportedCallType::Acompletion]),
        &call,
    );
    let dual = recording_dual(&recorder);
    let result = chat_result();
    store_response(
        &context,
        Some(&dual),
        &StoreRequest {
            result: &result,
            original_function: function,
            kwargs,
            args,
        },
    )
    .expect("the key write starts");
    context.tracker.wait_idle().await;
    single_write(&recorder).key
}

fn single_write(recorder: &Recorder) -> RecordedWrite {
    let writes = recorder.writes();
    assert_eq!(writes.len(), 1, "expected exactly one cache write");
    writes
        .into_iter()
        .next()
        .expect("the single write is present")
}
