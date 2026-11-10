#![cfg(test)]

use std::time::Duration;

use rstest::rstest;

use crate::replay::{Replayed, ResponseKind};
use crate::types::call_type::CachingSupportedCallType;

use super::shared::{
    Fake, MetadataRecord, convert, dict_of, messages_dict, non_stream_request, replay,
    responses_dict, stream_request,
};

#[rstest]
#[case::completion(CachingSupportedCallType::Completion)]
#[case::text_completion(CachingSupportedCallType::TextCompletion)]
#[case::embedding(CachingSupportedCallType::Embedding)]
#[case::rerank(CachingSupportedCallType::Rerank)]
#[case::transcription(CachingSupportedCallType::Transcription)]
#[case::responses(CachingSupportedCallType::Responses)]
fn metadata_cache_hit_is_stamped_on_every_converted_response(
    #[case] call_type: CachingSupportedCallType,
) {
    let fake = Fake::new();
    let outcome = replay(call_type, &non_stream_request(), &fake);
    let stamped = match &outcome {
        Replayed::Response(_, response)
        | Replayed::Transcription(response)
        | Replayed::Responses(response)
        | Replayed::Stream(response)
        | Replayed::ResponsesStream(response)
        | Replayed::MessagesStream(response) => response.cache_hit,
        Replayed::Unchanged(_) => false,
    };
    assert!(stamped);
}

#[test]
fn metadata_cache_hit_is_stamped_on_a_replayed_stream() {
    let fake = Fake::new();
    let outcome = replay(
        CachingSupportedCallType::Completion,
        &stream_request(),
        &fake,
    );
    let Replayed::Stream(stream) = outcome else {
        panic!("the cached completion did not replay as a stream");
    };
    assert!(stream.cache_hit);
}

#[test]
fn metadata_cache_hit_is_stamped_on_a_responses_stream() {
    let fake = Fake::new();
    let outcome = replay(
        CachingSupportedCallType::Responses,
        &stream_request(),
        &fake,
    );
    let Replayed::ResponsesStream(stream) = outcome else {
        panic!("the cached responses object did not replay as a stream");
    };
    assert!(stream.cache_hit);
    assert!(fake.calls().responses_stream_stamped);
}

#[test]
fn metadata_cache_hit_is_stamped_on_an_anthropic_messages_stream() {
    let fake = Fake::with_anthropic_events();
    let outcome = convert(
        messages_dict(),
        CachingSupportedCallType::AnthropicMessages,
        &non_stream_request(),
        &fake,
    )
    .unwrap();
    let Replayed::MessagesStream(stream) = outcome else {
        panic!("the cached anthropic events did not replay as a stream");
    };
    assert!(stream.cache_hit);
    assert_eq!(fake.calls().messages_dict, Some(dict_of(messages_dict())));
}

#[test]
fn metadata_an_unchanged_result_updates_metadata() {
    let fake = Fake::new();
    let outcome = convert(
        messages_dict(),
        CachingSupportedCallType::AnthropicMessages,
        &non_stream_request(),
        &fake,
    )
    .unwrap();
    let Replayed::Unchanged(value) = outcome else {
        panic!("the cached anthropic messages were converted without replay events");
    };
    assert_eq!(value, messages_dict());
    let calls = fake.calls();
    assert_eq!(calls.messages_probes, 1);
    assert_eq!(calls.messages_dict, Some(dict_of(messages_dict())));
    assert_eq!(calls.metadata.len(), 1);
}

#[test]
fn metadata_update_runs_once_with_the_model_and_elapsed_time() {
    let fake = Fake::new();
    convert(
        responses_dict(),
        CachingSupportedCallType::Aresponses,
        &non_stream_request(),
        &fake,
    )
    .unwrap();
    assert_eq!(
        fake.calls().metadata,
        vec![MetadataRecord {
            model: "gpt-4o".to_owned(),
            elapsed: Duration::from_millis(7),
        }]
    );
}

#[test]
fn metadata_the_typed_response_kind_is_recorded() {
    let fake = Fake::new();
    let outcome = replay(
        CachingSupportedCallType::Embedding,
        &non_stream_request(),
        &fake,
    );
    assert!(matches!(
        outcome,
        Replayed::Response(ResponseKind::Embedding, _)
    ));
}
