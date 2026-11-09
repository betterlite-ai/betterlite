#![cfg(test)]

use std::time::Duration;

use betterlite_context::local_ttl_cache::CacheValue;
use rstest::rstest;

use crate::replay::{ReplayRequest, Replayed, ResponseKind};
use crate::types::call_type::CachingSupportedCallType;

use super::shared::{
    Fake, HiddenRecord, chat_completion_dict, convert, embedding_dict, non_stream_request, replay,
    rerank_dict, stream_request, text_completion_dict, transcription_dict,
};

#[rstest]
#[case::completion(CachingSupportedCallType::Completion)]
#[case::acompletion(CachingSupportedCallType::Acompletion)]
fn conversion_completion_cached_dict_becomes_a_chat_response(
    #[case] call_type: CachingSupportedCallType,
) {
    let fake = Fake::new();
    let outcome = convert(
        chat_completion_dict(),
        call_type,
        &non_stream_request(),
        &fake,
    )
    .unwrap();
    let Replayed::Response(kind, response) = outcome else {
        panic!("a cached chat completion did not become a chat response");
    };
    assert_eq!(kind, ResponseKind::Chat);
    assert!(response.cache_hit);
    assert_eq!(fake.calls().model_kinds, vec![ResponseKind::Chat]);
}

#[rstest]
#[case::text_completion(CachingSupportedCallType::TextCompletion)]
#[case::atext_completion(CachingSupportedCallType::AtextCompletion)]
fn conversion_text_completion_cached_dict_becomes_a_text_response(
    #[case] call_type: CachingSupportedCallType,
) {
    let fake = Fake::new();
    let outcome = convert(
        text_completion_dict(),
        call_type,
        &non_stream_request(),
        &fake,
    )
    .unwrap();
    let Replayed::Response(kind, response) = outcome else {
        panic!("a cached text completion did not become a text completion response");
    };
    assert_eq!(kind, ResponseKind::Text);
    assert!(response.cache_hit);
    assert_eq!(fake.calls().model_kinds, vec![ResponseKind::Text]);
}

#[rstest]
#[case::embedding(CachingSupportedCallType::Embedding)]
#[case::aembedding(CachingSupportedCallType::Aembedding)]
fn conversion_embedding_cached_dict_becomes_an_embedding_response(
    #[case] call_type: CachingSupportedCallType,
) {
    let fake = Fake::new();
    let outcome = convert(embedding_dict(), call_type, &non_stream_request(), &fake).unwrap();
    let Replayed::Response(kind, response) = outcome else {
        panic!("a cached embedding did not become an embedding response");
    };
    assert_eq!(kind, ResponseKind::Embedding);
    assert!(response.cache_hit);
    assert_eq!(fake.calls().model_kinds, vec![ResponseKind::Embedding]);
}

#[rstest]
#[case::rerank(CachingSupportedCallType::Rerank)]
#[case::arerank(CachingSupportedCallType::Arerank)]
fn conversion_rerank_cached_dict_becomes_a_rerank_response(
    #[case] call_type: CachingSupportedCallType,
) {
    let fake = Fake::new();
    let outcome = convert(rerank_dict(), call_type, &non_stream_request(), &fake).unwrap();
    let Replayed::Response(kind, response) = outcome else {
        panic!("a cached rerank result did not become a rerank response");
    };
    assert_eq!(kind, ResponseKind::Rerank);
    assert!(response.cache_hit);
    assert_eq!(fake.calls().model_kinds, vec![ResponseKind::Rerank]);
}

#[rstest]
#[case::transcription(CachingSupportedCallType::Transcription)]
#[case::atranscription(CachingSupportedCallType::Atranscription)]
fn conversion_transcription_gets_whisper_hidden_params(
    #[case] call_type: CachingSupportedCallType,
) {
    let fake = Fake::new();
    let outcome = convert(
        transcription_dict(),
        call_type,
        &non_stream_request(),
        &fake,
    )
    .unwrap();
    let Replayed::Response(kind, response) = outcome else {
        panic!("a cached transcription did not become a transcription response");
    };
    assert_eq!(kind, ResponseKind::Transcription);
    assert!(response.cache_hit);
    let calls = fake.calls();
    assert_eq!(calls.transcription_calls, 1);
    assert_eq!(
        calls.transcription_hidden,
        Some(HiddenRecord {
            model: "whisper-1".to_owned(),
            custom_llm_provider: None,
            cache_hit: true,
        })
    );
}

#[test]
fn conversion_transcription_hidden_params_carry_the_requested_provider() {
    let fake = Fake::new();
    let request = ReplayRequest {
        stream: false,
        converted_stream_requested: false,
        agentic_loop_depth: 0,
        model: "azure/whisper",
        custom_llm_provider: Some("azure"),
        elapsed: Duration::from_millis(7),
    };
    convert(
        transcription_dict(),
        CachingSupportedCallType::Transcription,
        &request,
        &fake,
    )
    .unwrap();
    let hidden = fake.calls().transcription_hidden.unwrap();
    assert_eq!(hidden.custom_llm_provider, Some("azure".to_owned()));
}

#[rstest]
#[case::embedding(CachingSupportedCallType::Embedding)]
#[case::rerank(CachingSupportedCallType::Rerank)]
#[case::transcription(CachingSupportedCallType::Transcription)]
fn conversion_a_stream_request_still_returns_the_typed_response(
    #[case] call_type: CachingSupportedCallType,
) {
    let fake = Fake::new();
    let outcome = replay(call_type, &stream_request(), &fake);
    assert!(matches!(outcome, Replayed::Response(..)));
    assert!(fake.calls().wraps.is_empty());
}

#[test]
fn conversion_a_non_object_cached_value_is_returned_unchanged() {
    let fake = Fake::new();
    let cached = CacheValue::Integer(3);
    let outcome = convert(
        cached.clone(),
        CachingSupportedCallType::Completion,
        &non_stream_request(),
        &fake,
    )
    .unwrap();
    let Replayed::Unchanged(value) = outcome else {
        panic!("a non-object cached value was converted");
    };
    assert_eq!(value, cached);
    let calls = fake.calls();
    assert!(calls.model_kinds.is_empty());
    assert!(calls.wraps.is_empty());
    assert_eq!(calls.metadata.len(), 1);
}

#[test]
fn conversion_a_text_cached_value_is_returned_unchanged() {
    let fake = Fake::new();
    let cached = CacheValue::Text("not a response object".to_owned());
    let outcome = convert(
        cached,
        CachingSupportedCallType::Embedding,
        &non_stream_request(),
        &fake,
    )
    .unwrap();
    assert!(matches!(outcome, Replayed::Unchanged(_)));
    assert!(fake.calls().model_kinds.is_empty());
}
