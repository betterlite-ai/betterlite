use std::sync::Arc;

use super::support::{
    MODEL, ReadSupport, RecordingBackend, cache_text, chat_kwargs, handler, kwargs, source, text,
};
use crate::cache_key::value::KeyValue;
use crate::handler::kwargs::LOGGING_OBJECT_KWARG;
use crate::handler::response::CachingHandlerResponse;
use crate::handler::retrieve::{AsyncSupport, Retrieval, retrieve_from_cache};
use crate::types::call_type::CachingSupportedCallType;

#[test]
fn stores_request_kwargs_without_the_logging_object() {
    let request_kwargs = kwargs(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
        (LOGGING_OBJECT_KWARG, text("a-logging-object")),
    ]);
    let handler = handler(request_kwargs);
    assert!(!handler.request_kwargs.contains_key(LOGGING_OBJECT_KWARG));
    assert_eq!(handler.request_kwargs.get("model"), Some(&text(MODEL)));
    assert_eq!(handler.original_function.name, "acompletion");
    assert_eq!(handler.original_function.parameters, ["model", "messages"]);
    assert_eq!(handler.preset_cache_key, None);
    assert!(handler.async_streaming_chunks.is_empty());
    assert!(handler.sync_streaming_chunks.is_empty());
    assert!(handler.dual_cache.is_none());
}

#[test]
fn keeps_every_kwarg_when_no_logging_object_is_present() {
    let handler = handler(chat_kwargs());
    assert_eq!(handler.request_kwargs, chat_kwargs());
    assert!(handler.start_time.abs() < f64::EPSILON);
}

#[tokio::test]
async fn drops_the_logging_object_from_retrieved_kwargs() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = super::support::cache(Arc::clone(&backend));
    let request_kwargs = kwargs(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
        (LOGGING_OBJECT_KWARG, text("a-logging-object")),
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
    assert!(!handler.request_kwargs.contains_key(LOGGING_OBJECT_KWARG));
    assert_eq!(handler.request_kwargs.get("model"), Some(&text(MODEL)));
}

#[tokio::test]
async fn retrieval_does_not_keep_the_hit() {
    let backend = Arc::new(RecordingBackend::new(ReadSupport::Both));
    let cache = super::support::cache(Arc::clone(&backend));
    let mut handler = handler(chat_kwargs());
    let request_kwargs = chat_kwargs();
    let _miss = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Acompletion,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the first retrieval runs");
    let key = handler
        .preset_cache_key
        .clone()
        .expect("the first retrieval derives a preset key");
    backend.seed(&key, super::support::envelope(cache_text("cached-answer")));
    let hit = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Acompletion,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the second retrieval runs");
    assert_eq!(hit, Retrieval::Hit(cache_text("cached-answer")));
    backend.clear();
    let after = retrieve_from_cache(
        &mut handler,
        Some(&source(&cache, AsyncSupport::Supported)),
        CachingSupportedCallType::Acompletion,
        &request_kwargs,
        &[],
    )
    .await
    .expect("the third retrieval runs");
    assert_eq!(after, Retrieval::Miss);
}

#[test]
fn response_defaults_match_the_caching_handler_response() {
    let response = CachingHandlerResponse::default();
    assert_eq!(response.cached_result, None);
    assert_eq!(response.final_embedding_response, None);
    assert!(!response.all_embeddings_cached);
    assert_eq!(response.embedding_uncached_input, None);
}
