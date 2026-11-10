#![cfg(test)]

use std::time::Duration;

use betterlite_context::local_ttl_cache::CacheValue;
use rstest::rstest;

use crate::replay::{ReplayRequest, Replayed, ResponseKind};
use crate::types::call_type::CachingSupportedCallType;

use super::shared::{
    Fake, HiddenRecord, convert, dict_of, fixture, non_stream_request, replay, stream_request,
    transcription_dict,
};

#[rstest]
#[case::completion(CachingSupportedCallType::Completion, ResponseKind::Chat)]
#[case::acompletion(CachingSupportedCallType::Acompletion, ResponseKind::Chat)]
#[case::text_completion(CachingSupportedCallType::TextCompletion, ResponseKind::Text)]
#[case::atext_completion(CachingSupportedCallType::AtextCompletion, ResponseKind::Text)]
#[case::embedding(CachingSupportedCallType::Embedding, ResponseKind::Embedding)]
#[case::aembedding(CachingSupportedCallType::Aembedding, ResponseKind::Embedding)]
#[case::rerank(CachingSupportedCallType::Rerank, ResponseKind::Rerank)]
#[case::arerank(CachingSupportedCallType::Arerank, ResponseKind::Rerank)]
fn conversion_cached_dict_becomes_its_response_kind(
    #[case] call_type: CachingSupportedCallType,
    #[case] expected: ResponseKind,
) {
    let fake = Fake::new();
    let cached = dict_of(fixture(call_type));
    let outcome = convert(fixture(call_type), call_type, &non_stream_request(), &fake).unwrap();
    let Replayed::Response(kind, response) = outcome else {
        panic!("the cached result did not replay as a model response");
    };
    assert_eq!(kind, expected);
    assert!(response.cache_hit);
    let calls = fake.calls();
    assert_eq!(calls.model_kinds, vec![expected]);
    assert_eq!(calls.model_dicts, vec![cached]);
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
    let Replayed::Transcription(response) = outcome else {
        panic!("the cached transcription did not replay as a transcription response");
    };
    assert!(response.cache_hit);
    let calls = fake.calls();
    assert_eq!(calls.transcription_calls, 1);
    assert_eq!(
        calls.transcription_dict,
        Some(dict_of(transcription_dict()))
    );
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
    assert_eq!(hidden.model, "whisper-1");
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
    let streamed = matches!(
        outcome,
        Replayed::Stream(_) | Replayed::ResponsesStream(_) | Replayed::MessagesStream(_)
    );
    assert!(!streamed);
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
