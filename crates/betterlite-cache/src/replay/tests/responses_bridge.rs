#![cfg(test)]

use rstest::rstest;

use betterlite_context::local_ttl_cache::CacheValue;

use crate::replay::{CachedDict, Replayed, ResponseKind, is_chat_completion_cached_dict};
use crate::types::call_type::CachingSupportedCallType;

use super::shared::{
    Fake, WrapRecord, chat_completion_dict, convert, dict, dict_of, non_stream_request,
    responses_dict, stream_request, text,
};

#[derive(Clone, Copy, Debug)]
enum Shape {
    ChatcmplId,
    ChatCompletionObject,
    ChunkObject,
    ChoicesOnly,
    ObjectBeatsChoices,
    OtherIdWithChoices,
    NonStringIdWithChoices,
    IdBeatsObject,
    Empty,
}

fn shaped(kind: Shape) -> CachedDict {
    let value = match kind {
        Shape::ChatcmplId => dict(vec![("id", text("chatcmpl-1"))]),
        Shape::ChatCompletionObject => dict(vec![
            ("id", text("resp_1")),
            ("object", text("chat.completion")),
        ]),
        Shape::ChunkObject => dict(vec![("object", text("chat.completion.chunk"))]),
        Shape::ChoicesOnly => dict(vec![("choices", CacheValue::Array(Vec::new()))]),
        Shape::ObjectBeatsChoices => dict(vec![
            ("object", text("response")),
            ("choices", CacheValue::Array(Vec::new())),
        ]),
        Shape::OtherIdWithChoices => dict(vec![
            ("id", text("resp_1")),
            ("choices", CacheValue::Array(Vec::new())),
        ]),
        Shape::NonStringIdWithChoices => dict(vec![
            ("id", CacheValue::Integer(1)),
            ("choices", CacheValue::Array(Vec::new())),
        ]),
        Shape::IdBeatsObject => dict(vec![
            ("id", text("chatcmpl-1")),
            ("object", text("response")),
        ]),
        Shape::Empty => dict(Vec::new()),
    };
    dict_of(value)
}

#[rstest]
#[case::chatcmpl_id(Shape::ChatcmplId, true)]
#[case::chat_completion_object(Shape::ChatCompletionObject, true)]
#[case::chunk_object(Shape::ChunkObject, true)]
#[case::choices_only(Shape::ChoicesOnly, true)]
#[case::object_without_prefix_beats_choices(Shape::ObjectBeatsChoices, false)]
#[case::other_id_with_choices(Shape::OtherIdWithChoices, true)]
#[case::non_string_id_with_choices(Shape::NonStringIdWithChoices, true)]
#[case::id_beats_object(Shape::IdBeatsObject, true)]
#[case::empty(Shape::Empty, false)]
fn responses_bridge_chat_shape_detection(#[case] kind: Shape, #[case] expected: bool) {
    assert_eq!(is_chat_completion_cached_dict(&shaped(kind)), expected);
}

#[rstest]
#[case::responses(CachingSupportedCallType::Responses)]
#[case::aresponses(CachingSupportedCallType::Aresponses)]
fn responses_bridge_chat_shaped_cache_becomes_a_chat_response(
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
    assert!(matches!(outcome, Replayed::Response(ResponseKind::Chat, _)));
    let calls = fake.calls();
    assert_eq!(calls.model_kinds, vec![ResponseKind::Chat]);
    assert_eq!(calls.responses_calls, 0);
    assert_eq!(calls.responses_stream_calls, 0);
}

#[test]
fn responses_bridge_choices_only_cache_becomes_a_chat_response() {
    let fake = Fake::new();
    let cached = dict(vec![("choices", CacheValue::Array(Vec::new()))]);
    let outcome = convert(
        cached,
        CachingSupportedCallType::Responses,
        &non_stream_request(),
        &fake,
    )
    .unwrap();
    assert!(matches!(outcome, Replayed::Response(ResponseKind::Chat, _)));
    assert_eq!(fake.calls().model_kinds, vec![ResponseKind::Chat]);
}

#[rstest]
#[case::responses(CachingSupportedCallType::Responses, false)]
#[case::aresponses(CachingSupportedCallType::Aresponses, true)]
fn responses_bridge_chat_shaped_stream_uses_the_bridged_converter(
    #[case] call_type: CachingSupportedCallType,
    #[case] asynchronous: bool,
) {
    let fake = Fake::new();
    let outcome = convert(chat_completion_dict(), call_type, &stream_request(), &fake).unwrap();
    assert!(matches!(outcome, Replayed::Stream(_)));
    let calls = fake.calls();
    assert_eq!(
        calls.wraps,
        vec![WrapRecord {
            asynchronous,
            model: "gpt-4o".to_owned(),
            custom_llm_provider: "cached_response".to_owned(),
        }]
    );
    assert_eq!(calls.wrapper_dict, Some(dict_of(chat_completion_dict())));
    assert!(calls.model_kinds.is_empty());
    assert_eq!(calls.responses_calls, 0);
}

#[rstest]
#[case::responses_object(CachingSupportedCallType::Responses, false)]
#[case::aresponses_object(CachingSupportedCallType::Aresponses, false)]
#[case::responses_stream(CachingSupportedCallType::Responses, true)]
#[case::aresponses_stream(CachingSupportedCallType::Aresponses, true)]
fn responses_bridge_other_cache_replays_as_a_responses_result(
    #[case] call_type: CachingSupportedCallType,
    #[case] stream: bool,
) {
    let fake = Fake::new();
    let request = if stream {
        stream_request()
    } else {
        non_stream_request()
    };
    let expected = dict_of(responses_dict());
    let outcome = convert(responses_dict(), call_type, &request, &fake).unwrap();
    let calls = fake.calls();
    assert_eq!(calls.responses_dict, Some(expected));
    if stream {
        let Replayed::ResponsesStream(replayed) = outcome else {
            panic!("a non-chat cached responses result did not replay as a stream");
        };
        assert!(replayed.cache_hit);
        assert!(calls.responses_stream_stamped);
        assert_eq!(calls.responses_stream_calls, 1);
        assert_eq!(calls.responses_stream_call_type, Some(call_type));
        assert_eq!(calls.responses_calls, 1);
    } else {
        let Replayed::Responses(replayed) = outcome else {
            panic!("a non-chat cached responses result did not become a responses object");
        };
        assert!(replayed.cache_hit);
        assert_eq!(calls.responses_calls, 1);
        assert_eq!(calls.responses_stream_calls, 0);
    }
    assert!(calls.wraps.is_empty());
}
