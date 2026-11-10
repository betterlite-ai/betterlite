#![cfg(test)]

use rstest::rstest;

use crate::replay::ReplayedBy;
use crate::types::call_type::CachingSupportedCallType;

use super::shared::{Fake, convert, messages_dict, non_stream_request, replay, stream_request};

#[derive(Clone, Copy, Debug)]
enum StreamKind {
    Chat,
    Responses,
    Messages,
}

fn stream_outcome(kind: StreamKind) -> ReplayedBy<Fake> {
    match kind {
        StreamKind::Chat => replay(
            CachingSupportedCallType::Completion,
            &stream_request(),
            &Fake::new(),
        ),
        StreamKind::Responses => replay(
            CachingSupportedCallType::Responses,
            &stream_request(),
            &Fake::new(),
        ),
        StreamKind::Messages => {
            let fake = Fake::with_anthropic_events();
            convert(
                messages_dict(),
                CachingSupportedCallType::AnthropicMessages,
                &non_stream_request(),
                &fake,
            )
            .unwrap()
        }
    }
}

#[rstest]
#[case::custom_stream_wrapper(StreamKind::Chat)]
#[case::responses_iterator(StreamKind::Responses)]
#[case::anthropic_messages_iterator(StreamKind::Messages)]
fn deferral_each_stream_container_defers_success_callbacks(#[case] kind: StreamKind) {
    assert!(stream_outcome(kind).defers_success_callbacks());
}

#[rstest]
#[case::completion(CachingSupportedCallType::Completion)]
#[case::acompletion(CachingSupportedCallType::Acompletion)]
#[case::text_completion(CachingSupportedCallType::TextCompletion)]
#[case::atext_completion(CachingSupportedCallType::AtextCompletion)]
#[case::embedding(CachingSupportedCallType::Embedding)]
#[case::aembedding(CachingSupportedCallType::Aembedding)]
#[case::rerank(CachingSupportedCallType::Rerank)]
#[case::arerank(CachingSupportedCallType::Arerank)]
#[case::transcription(CachingSupportedCallType::Transcription)]
#[case::atranscription(CachingSupportedCallType::Atranscription)]
#[case::responses(CachingSupportedCallType::Responses)]
#[case::aresponses(CachingSupportedCallType::Aresponses)]
#[case::anthropic_messages(CachingSupportedCallType::AnthropicMessages)]
#[case::aanthropic_messages(CachingSupportedCallType::AanthropicMessages)]
fn deferral_a_non_stream_result_runs_callbacks_now(#[case] call_type: CachingSupportedCallType) {
    let fake = Fake::new();
    let outcome = replay(call_type, &non_stream_request(), &fake);
    assert!(!outcome.defers_success_callbacks());
}

#[test]
fn deferral_an_unchanged_cached_value_runs_callbacks_now() {
    let fake = Fake::new();
    let outcome = convert(
        messages_dict(),
        CachingSupportedCallType::AanthropicMessages,
        &non_stream_request(),
        &fake,
    )
    .unwrap();
    assert!(!outcome.defers_success_callbacks());
}
