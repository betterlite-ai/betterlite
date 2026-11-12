use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheValue;
use rstest::rstest;

use super::support::{
    MODEL, Read, ReadKind, ReadSupport, RecordingBackend, TestCache, cache as build_cache,
    cache_object, cache_text, cache_with_mode, cached_embedding, chat_kwargs, embedding_kwargs,
    envelope, handler, handler_named, integer, key_request_of, kwargs, object, source, text,
};
use crate::cache_key::derive::get_cache_key;
use crate::cache_key::value::{KeyObject, KeyValue};
use crate::handler::HandlerError;
use crate::handler::embedding::embedding_entry_key;
use crate::handler::retrieve::{AsyncSupport, Retrieval, retrieve_from_cache};
use crate::types::cache_type::CacheMode;
use crate::types::call_type::CachingSupportedCallType;

fn expected_key(entries: &[(&str, KeyValue)]) -> String {
    let cache = build_cache(Arc::new(RecordingBackend::new(ReadSupport::Both)));
    get_cache_key(
        &mut key_request_of(entries),
        &cache.key_settings,
        &cache.hooks,
    )
    .expect("the expected key derives")
}

fn stored_value() -> CacheValue {
    cache_object(&[("id", cache_text("cached-answer"))])
}

fn seed_embedding_input(
    cache: &TestCache,
    backend: &RecordingBackend,
    request_kwargs: &KeyObject,
    input: &KeyValue,
    record: CacheValue,
) {
    let key = embedding_entry_key(cache, request_kwargs, input).expect("the entry key derives");
    backend.seed(&key, envelope(record));
}

#[tokio::test]
async fn single_call_computes_a_preset_key_when_absent() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let mut handler = handler(chat_kwargs());
    let result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Acompletion,
        &chat_kwargs(),
        &[],
    )
    .await
    .expect("the retrieval runs");
    assert_eq!(result, Retrieval::Miss);
    let expected = expected_key(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
    ]);
    assert_eq!(handler.preset_cache_key.as_deref(), Some(expected.as_str()));
    let reads = backend.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].kind, ReadKind::Async);
    assert_eq!(reads[0].key, expected);
}

#[tokio::test]
async fn caller_cache_key_wins_and_stays_in_request_kwargs() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let request_kwargs = kwargs(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
        ("cache_key", text("caller-key")),
    ]);
    let mut handler = handler(request_kwargs.clone());
    let _result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Acompletion,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the retrieval runs");
    assert_eq!(handler.preset_cache_key.as_deref(), Some("caller-key"));
    assert_eq!(
        handler.request_kwargs.get("cache_key"),
        Some(&text("caller-key"))
    );
    assert_eq!(backend.reads()[0].key, "caller-key");
}

#[tokio::test]
async fn stream_precomputes_the_cache_key_before_the_read() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let request_kwargs = kwargs(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
        ("stream", KeyValue::Bool(true)),
    ]);
    let mut handler = handler(request_kwargs.clone());
    let _result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Acompletion,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the retrieval runs");
    let expected = expected_key(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
        ("stream", KeyValue::Bool(true)),
    ]);
    assert_eq!(handler.preset_cache_key.as_deref(), Some(expected.as_str()));
    assert_eq!(
        handler.request_kwargs.get("cache_key"),
        Some(&KeyValue::String(expected.clone()))
    );
    assert_eq!(backend.reads()[0].key, expected);
}

#[tokio::test]
async fn stream_with_a_caller_key_keeps_the_caller_key() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let request_kwargs = kwargs(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
        ("stream", KeyValue::Bool(true)),
        ("cache_key", text("stream-key")),
    ]);
    let mut handler = handler(request_kwargs.clone());
    let _result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Acompletion,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the retrieval runs");
    assert_eq!(handler.preset_cache_key.as_deref(), Some("stream-key"));
    assert_eq!(backend.reads()[0].key, "stream-key");
}

#[tokio::test]
async fn null_metadata_is_removed_before_the_read() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let request_kwargs = kwargs(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
        ("metadata", KeyValue::Null),
    ]);
    let mut handler = handler(request_kwargs.clone());
    let _result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Acompletion,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the retrieval runs");
    assert!(!handler.request_kwargs.contains_key("metadata"));
}

#[tokio::test]
async fn positional_args_are_merged_over_kwargs() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let request_kwargs = kwargs(&[
        ("model", text("old-model")),
        ("messages", KeyValue::Array(Vec::new())),
    ]);
    let mut handler = handler_named(
        "acompletion",
        &["model", "messages"],
        request_kwargs.clone(),
    );
    let new_messages = KeyValue::Array(vec![object(&[("role", text("user"))])]);
    let args = [text("new-model"), new_messages.clone()];
    let _result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Acompletion,
        &request_kwargs,
        &args,
    )
    .await
    .expect("the retrieval runs");
    assert_eq!(
        handler.request_kwargs.get("model"),
        Some(&text("new-model"))
    );
    assert_eq!(handler.request_kwargs.get("messages"), Some(&new_messages));
}

#[tokio::test]
async fn arguments_past_the_signature_are_dropped() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let request_kwargs = chat_kwargs();
    let mut handler = handler_named("acompletion", &["model"], request_kwargs.clone());
    let _result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Acompletion,
        &request_kwargs,
        &[text("first"), text("second")],
    )
    .await
    .expect("the retrieval runs");
    assert_eq!(handler.request_kwargs.get("model"), Some(&text("first")));
}

#[tokio::test]
async fn cached_value_is_returned_as_a_hit() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let expected = expected_key(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
    ]);
    backend.seed(&expected, envelope(stored_value()));
    let mut handler = handler(chat_kwargs());
    let result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Acompletion,
        &chat_kwargs(),
        &[],
    )
    .await
    .expect("the retrieval runs");
    assert_eq!(result, Retrieval::Hit(stored_value()));
    assert_eq!(handler.preset_cache_key.as_deref(), Some(expected.as_str()));
}

#[tokio::test]
async fn sync_fallback_reads_when_async_is_unsupported() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::SyncOnly));
    let cache = build_cache(Arc::clone(&backend));
    let expected = expected_key(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
    ]);
    backend.seed(&expected, envelope(stored_value()));
    let mut handler = handler(chat_kwargs());
    let result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Unsupported)),
        CachingSupportedCallType::Acompletion,
        &chat_kwargs(),
        &[],
    )
    .await
    .expect("the retrieval runs");
    assert_eq!(result, Retrieval::Hit(stored_value()));
    assert_eq!(
        backend.reads(),
        [Read {
            kind: ReadKind::Sync,
            key: expected,
        }]
    );
}

#[tokio::test]
async fn async_read_is_used_when_supported() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::AsyncOnly));
    let cache = build_cache(Arc::clone(&backend));
    let expected = expected_key(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
    ]);
    backend.seed(&expected, envelope(stored_value()));
    let mut handler = handler(chat_kwargs());
    let result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Acompletion,
        &chat_kwargs(),
        &[],
    )
    .await
    .expect("the retrieval runs");
    assert_eq!(result, Retrieval::Hit(stored_value()));
    assert_eq!(backend.reads()[0].kind, ReadKind::Async);
}

#[tokio::test]
async fn no_cache_is_a_miss_without_touching_request_kwargs() {
    let request_kwargs = kwargs(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
        ("metadata", KeyValue::Null),
    ]);
    let mut handler = handler(request_kwargs.clone());
    let result =
        retrieve_from_cache::<super::support::SilentLogger, super::support::SilentConsole>(
            &mut handler,
            None,
            CachingSupportedCallType::Acompletion,
            &request_kwargs,
            &[],
        )
        .await
        .expect("no cache never fails");
    assert_eq!(result, Retrieval::Miss);
    assert_eq!(handler.request_kwargs, request_kwargs);
}

#[rstest]
#[case::wrong_stream_type(kwargs(&[("model", text(MODEL)), ("messages", KeyValue::Array(Vec::new())), ("stream", text("yes"))]))]
#[case::wrong_cache_key_type(kwargs(&[("model", text(MODEL)), ("messages", KeyValue::Array(Vec::new())), ("cache_key", integer(7))]))]
#[case::wrong_metadata_type(kwargs(&[("model", text(MODEL)), ("messages", KeyValue::Array(Vec::new())), ("metadata", text("bad"))]))]
#[case::wrong_cache_type(kwargs(&[("model", text(MODEL)), ("messages", KeyValue::Array(Vec::new())), ("cache", text("bad"))]))]
#[case::wrong_control_type(kwargs(&[("model", text(MODEL)), ("messages", KeyValue::Array(Vec::new())), ("cache", object(&[("no-store", text("yes"))]))]))]
#[tokio::test]
async fn malformed_control_kwargs_are_rejected(#[case] request_kwargs: KeyObject) {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let mut handler = handler(request_kwargs.clone());
    let result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Acompletion,
        &request_kwargs,
        &[],
    )
    .await;
    assert!(matches!(result, Err(HandlerError::FieldType { .. })));
    assert_eq!(handler.preset_cache_key, None);
    assert!(backend.reads().is_empty());
}

#[tokio::test]
async fn embedding_hits_are_validated_per_input() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let request_kwargs = embedding_kwargs(&["alpha", "beta", "gamma"]);
    seed_embedding_input(
        &cache,
        &backend,
        &request_kwargs,
        &text("alpha"),
        cached_embedding(2),
    );
    seed_embedding_input(
        &cache,
        &backend,
        &request_kwargs,
        &text("beta"),
        cached_embedding(1),
    );
    let mut handler = handler_named("aembedding", &["model", "input"], request_kwargs.clone());
    let result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Aembedding,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the retrieval runs");
    match result {
        Retrieval::Embeddings(items) => {
            assert_eq!(items.len(), 3);
            assert!(items[0].is_some());
            assert!(items[1].is_none());
            assert!(items[2].is_none());
        }
        Retrieval::Miss | Retrieval::Hit(_) => panic!("expected per-input embedding hits"),
    }
    assert_eq!(handler.preset_cache_key, None);
    assert_eq!(backend.reads().len(), 3);
}

#[tokio::test]
async fn embedding_with_every_input_missing_is_a_miss() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let request_kwargs = embedding_kwargs(&["alpha", "beta"]);
    let mut handler = handler_named("aembedding", &["model", "input"], request_kwargs.clone());
    let result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Aembedding,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the retrieval runs");
    assert_eq!(result, Retrieval::Miss);
    assert_eq!(backend.reads().len(), 2);
}

#[tokio::test]
async fn embedding_string_input_reads_one_key() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let request_kwargs = kwargs(&[
        ("model", text("text-embedding-3-small")),
        ("input", text("alpha")),
    ]);
    seed_embedding_input(
        &cache,
        &backend,
        &request_kwargs,
        &text("alpha"),
        cached_embedding(2),
    );
    let mut handler = handler_named("aembedding", &["model", "input"], request_kwargs.clone());
    let result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Aembedding,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the retrieval runs");
    match result {
        Retrieval::Embeddings(items) => assert_eq!(items.len(), 1),
        Retrieval::Miss | Retrieval::Hit(_) => panic!("expected one embedding hit"),
    }
    assert_eq!(backend.reads().len(), 1);
    assert_eq!(handler.request_kwargs.get("input"), Some(&text("alpha")));
}

#[tokio::test]
async fn embedding_token_list_input_is_read_by_its_own_key() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let tokens = KeyValue::Array(vec![integer(1), integer(2)]);
    let request_kwargs = kwargs(&[
        ("model", text("text-embedding-3-small")),
        ("input", KeyValue::Array(vec![tokens.clone()])),
    ]);
    seed_embedding_input(
        &cache,
        &backend,
        &request_kwargs,
        &tokens,
        cached_embedding(2),
    );
    let mut handler = handler_named("aembedding", &["model", "input"], request_kwargs.clone());
    let result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Aembedding,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the retrieval runs");
    match result {
        Retrieval::Embeddings(items) => assert!(items[0].is_some()),
        Retrieval::Miss | Retrieval::Hit(_) => panic!("expected a token embedding hit"),
    }
}

#[tokio::test]
async fn flat_token_array_input_is_read_as_one_input() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let tokens = KeyValue::Array(vec![integer(1), integer(2)]);
    let request_kwargs = kwargs(&[
        ("model", text("text-embedding-3-small")),
        ("input", tokens.clone()),
    ]);
    seed_embedding_input(
        &cache,
        &backend,
        &request_kwargs,
        &tokens,
        cached_embedding(2),
    );
    let mut handler = handler_named("aembedding", &["model", "input"], request_kwargs.clone());
    let result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Aembedding,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the retrieval runs");
    match result {
        Retrieval::Embeddings(items) => {
            assert_eq!(items.len(), 1);
            assert!(items[0].is_some());
        }
        Retrieval::Miss | Retrieval::Hit(_) => panic!("expected one token embedding hit"),
    }
    assert_eq!(backend.reads().len(), 1);
}

#[tokio::test]
async fn embedding_read_ignores_s_maxage() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let request_kwargs = kwargs(&[
        ("model", text("text-embedding-3-small")),
        ("input", text("alpha")),
        ("cache", object(&[("s-maxage", integer(1))])),
    ]);
    seed_embedding_input(
        &cache,
        &backend,
        &request_kwargs,
        &text("alpha"),
        cached_embedding(2),
    );
    let mut handler = handler_named("aembedding", &["model", "input"], request_kwargs.clone());
    let result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Aembedding,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the retrieval runs");
    match result {
        Retrieval::Embeddings(items) => assert!(items[0].is_some()),
        Retrieval::Miss | Retrieval::Hit(_) => panic!("expected the stale entry to hit"),
    }
}

#[tokio::test]
async fn embedding_read_ignores_use_cache_in_default_off() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = cache_with_mode(Arc::clone(&backend), CacheMode::DefaultOff);
    let request_kwargs = kwargs(&[
        ("model", text("text-embedding-3-small")),
        ("input", text("alpha")),
        ("cache", object(&[("use-cache", KeyValue::Bool(true))])),
    ]);
    seed_embedding_input(
        &cache,
        &backend,
        &request_kwargs,
        &text("alpha"),
        cached_embedding(2),
    );
    let mut handler = handler_named("aembedding", &["model", "input"], request_kwargs.clone());
    let result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Aembedding,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the retrieval runs");
    assert_eq!(result, Retrieval::Miss);
    assert!(backend.reads().is_empty());
}

#[rstest]
#[case::missing_input(kwargs(&[("model", text("text-embedding-3-small"))]))]
#[case::number_input(kwargs(&[("model", text("text-embedding-3-small")), ("input", integer(5))]))]
#[case::object_input(kwargs(&[("model", text("text-embedding-3-small")), ("input", object(&[("text", text("alpha"))]))]))]
#[case::array_of_strings_and_numbers(kwargs(&[("model", text("text-embedding-3-small")), ("input", KeyValue::Array(vec![text("alpha"), integer(5)]))]))]
#[tokio::test]
async fn malformed_embedding_input_is_rejected(#[case] request_kwargs: KeyObject) {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = build_cache(Arc::clone(&backend));
    let mut handler = handler_named("aembedding", &["model", "input"], request_kwargs.clone());
    let result = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Aembedding,
        &request_kwargs,
        &[],
    )
    .await;
    assert!(matches!(
        result,
        Err(HandlerError::FieldType { field: "input", .. })
    ));
    assert!(backend.reads().is_empty());
}
