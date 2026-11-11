use betterlite_models::decisions::Json;
use std::iter::{empty, once};
use std::time::Duration;

use futures_util::stream::iter as stream_iter;
use futures_util::{StreamExt, TryStreamExt as _};
use tokio::time::Instant;

use super::support;
use crate::message_utilities::response_streams::StreamError;
use crate::message_utilities::response_streams::iterators::{
    DeltaIdentity, ModelResponseIterator, ModelResponseListIterator,
};
use crate::message_utilities::response_streams::text_completion::TextCompletionStreamWrapper;

const TEXT_COMPLETION: &str = r#"{
    "id": "chatcmpl-1",
    "object": "text_completion",
    "created": 123,
    "model": null,
    "choices": [{"finish_reason": null, "index": 0, "text": "hi", "logprobs": null, "reasoning_content": null}],
    "usage": {"completion_tokens": 0, "prompt_tokens": 0, "total_tokens": 0, "completion_tokens_details": null, "prompt_tokens_details": null}
}"#;

const RESPONSE: &str = r#"{
    "id": "r1",
    "created": 42,
    "model": "gpt-4o",
    "object": "chat.completion",
    "system_fingerprint": null,
    "choices": [{"finish_reason": "stop", "index": 0, "message": {"content": "hello", "role": "assistant", "tool_calls": null, "function_call": null}}]
}"#;

fn chunks(texts: &[&str]) -> Vec<Json> {
    texts
        .iter()
        .map(|content| {
            support::value(&format!(
                r#"{{"id": "chatcmpl-1", "created": 123, "object": "chat.completion.chunk", "choices": [{{"finish_reason": null, "index": 0, "delta": {{"content": "{content}"}}, "logprobs": null}}]}}"#
            ))
        })
        .collect()
}

#[tokio::test]
async fn text_completion_stream_converts_every_chunk() {
    let wrapper = TextCompletionStreamWrapper {
        chunks: chunks(&["hi"]).into_iter(),
        stream_options: None,
    };
    let converted = wrapper.collect::<Result<Vec<Json>, StreamError>>().unwrap();
    assert_eq!(converted, support::values(&format!("[{TEXT_COMPLETION}]")));
}

#[tokio::test]
async fn text_completion_stream_reports_a_missing_chunk() {
    let wrapper = TextCompletionStreamWrapper {
        chunks: once(support::value("\"None\"")),
        stream_options: None,
    };
    let outcome = wrapper.collect::<Result<Vec<Json>, StreamError>>();
    assert_eq!(outcome, Err(StreamError::MissingChunk));
}

#[rstest::rstest]
#[case::include(
    r#"{"include_usage": true}"#,
    r#""usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}"#
)]
#[case::exclude(r#"{"include_usage": false}"#, r#""usage": {"completion_tokens": 0, "prompt_tokens": 0, "total_tokens": 0, "completion_tokens_details": null, "prompt_tokens_details": null}"#)]
#[tokio::test]
async fn text_completion_stream_usage_follows_the_option(
    #[case] options: &str,
    #[case] usage: &str,
) {
    let chunk = support::value(
        r#"{"id": "x", "created": 1, "model": "m", "choices": [{"finish_reason": "stop", "index": 2, "delta": {"content": "a", "reasoning_content": "think"}}], "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}}"#,
    );
    let wrapper = TextCompletionStreamWrapper {
        chunks: once(chunk),
        stream_options: Some(support::value(options)),
    };
    let converted: Vec<Json> = wrapper.collect::<Result<_, StreamError>>().unwrap();
    let expected = support::value(&format!(
        r#"{{"id": "x", "object": "text_completion", "created": 1, "model": "m", "choices": [{{"finish_reason": "stop", "index": 2, "text": "a", "logprobs": null, "reasoning_content": "think"}}], {usage}}}"#
    ));
    assert_eq!(converted, vec![expected]);
}

#[tokio::test]
async fn text_completion_stream_defaults_an_absent_delta_content_index_and_finish_reason() {
    let finish = support::value(
        r#"{"id": "chatcmpl-1", "created": 123, "model": "gpt-4o", "choices": [{"finish_reason": "stop", "delta": {}}]}"#,
    );
    let bare = support::value(r#"{"id": "chatcmpl-1", "choices": [{"delta": {}}]}"#);
    let wrapper = TextCompletionStreamWrapper {
        chunks: vec![finish, bare].into_iter(),
        stream_options: None,
    };
    let converted: Vec<Json> = wrapper.collect::<Result<_, StreamError>>().unwrap();
    assert_eq!(
        converted,
        support::values(
            r#"[
    {"id": "chatcmpl-1", "object": "text_completion", "created": 123, "model": "gpt-4o", "choices": [{"finish_reason": "stop", "index": 0, "text": null, "logprobs": null, "reasoning_content": null}], "usage": {"completion_tokens": 0, "prompt_tokens": 0, "total_tokens": 0, "completion_tokens_details": null, "prompt_tokens_details": null}},
    {"id": "chatcmpl-1", "object": "text_completion", "created": null, "model": null, "choices": [{"finish_reason": null, "index": 0, "text": null, "logprobs": null, "reasoning_content": null}], "usage": {"completion_tokens": 0, "prompt_tokens": 0, "total_tokens": 0, "completion_tokens_details": null, "prompt_tokens_details": null}}
]"#
        )
    );
}

#[tokio::test]
async fn text_completion_stream_rejects_a_chunk_without_choices() {
    let wrapper = TextCompletionStreamWrapper {
        chunks: once(support::value(r#"{"id": "x"}"#)),
        stream_options: None,
    };
    assert_eq!(
        wrapper.collect::<Result<Vec<Json>, StreamError>>(),
        Err(StreamError::NoChoices)
    );
}

#[tokio::test]
async fn text_completion_stream_async_yields_the_same_chunks() {
    let input: Vec<Json> = chunks(&["hi"]);
    let wrapper = TextCompletionStreamWrapper {
        chunks: stream_iter(input),
        stream_options: None,
    };
    let converted: Vec<Json> = wrapper.try_collect().await.unwrap();
    assert_eq!(converted, support::values(&format!("[{TEXT_COMPLETION}]")));
}

#[tokio::test]
async fn model_response_iterator_yields_the_response_once() {
    let response = support::value(RESPONSE);
    let mut iterator = ModelResponseIterator::new(response.clone(), None).unwrap();
    assert_eq!(Iterator::next(&mut iterator), Some(response));
    assert_eq!(Iterator::next(&mut iterator), None);
}

#[tokio::test]
async fn model_response_iterator_converts_to_a_delta_chunk() {
    let response = support::value(RESPONSE);
    let identity = DeltaIdentity {
        id: "delta-1",
        created: 7,
    };
    let mut iterator = ModelResponseIterator::new(response, Some(identity)).unwrap();
    assert_eq!(
        Iterator::next(&mut iterator),
        Some(support::value(
            r#"{"id": "delta-1", "created": 7, "object": "chat.completion.chunk", "choices": [{"finish_reason": null, "index": 0, "delta": {"content": "hello", "role": null, "function_call": null, "tool_calls": null, "audio": null}, "logprobs": null}], "provider_specific_fields": null}"#
        ))
    );
    assert_eq!(Iterator::next(&mut iterator), None);
}

#[tokio::test]
async fn model_response_iterator_rejects_a_response_without_choices() {
    let response = support::value(r#"{"id": "r1"}"#);
    let identity = DeltaIdentity {
        id: "delta-1",
        created: 7,
    };
    assert!(matches!(
        ModelResponseIterator::new(response, Some(identity)),
        Err(StreamError::NoChoices)
    ));
}

#[tokio::test]
async fn model_response_list_iterator_yields_each_response() {
    let first = support::value(r#"{"id": "a"}"#);
    let second = support::value(r#"{"id": "b"}"#);
    let iterator =
        ModelResponseListIterator::new(vec![first.clone(), second.clone()], None).unwrap();
    assert_eq!(
        Iterator::collect::<Vec<Json>>(iterator),
        vec![first, second]
    );
}

#[tokio::test]
async fn model_response_list_iterator_ends_after_the_last_response() {
    let mut iterator = ModelResponseListIterator::new(Vec::new(), Some(0.0_f64)).unwrap();
    assert_eq!(Iterator::next(&mut iterator), None);
}

#[rstest::rstest]
#[case::without_delay(None)]
#[case::with_a_zero_delay(Some(0.0_f64))]
#[case::with_a_short_delay(Some(0.001_f64))]
#[tokio::test]
async fn model_response_list_iterator_streams_each_response(#[case] delay: Option<f64>) {
    let first = support::value(r#"{"id": "a"}"#);
    let second = support::value(r#"{"id": "b"}"#);
    let mut stream =
        ModelResponseListIterator::new(vec![first.clone(), second.clone()], delay).unwrap();
    assert_eq!(StreamExt::next(&mut stream).await, Some(first));
    assert_eq!(StreamExt::next(&mut stream).await, Some(second));
    assert_eq!(StreamExt::next(&mut stream).await, None);
}

#[tokio::test]
async fn model_response_list_iterator_rejects_a_negative_delay() {
    assert!(matches!(
        ModelResponseListIterator::new(Vec::new(), Some(-1.0_f64)),
        Err(StreamError::Delay { .. })
    ));
}

#[tokio::test]
async fn text_completion_stream_has_no_output_for_no_chunks() {
    let wrapper = TextCompletionStreamWrapper {
        chunks: empty(),
        stream_options: None,
    };
    assert_eq!(
        wrapper.collect::<Result<Vec<Json>, StreamError>>().unwrap(),
        Vec::new()
    );
}

#[tokio::test(start_paused = true)]
async fn model_response_list_iterator_waits_for_the_delay_before_each_response() {
    let first = support::value(r#"{"id": "a"}"#);
    let second = support::value(r#"{"id": "b"}"#);
    let mut stream =
        ModelResponseListIterator::new(vec![first.clone(), second.clone()], Some(2.0_f64)).unwrap();
    let started = Instant::now();
    assert_eq!(StreamExt::next(&mut stream).await, Some(first));
    assert_eq!(started.elapsed(), Duration::from_secs(2));
    assert_eq!(StreamExt::next(&mut stream).await, Some(second));
    assert_eq!(started.elapsed(), Duration::from_secs(4));
}
