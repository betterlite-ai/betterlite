#![cfg(test)]

use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_translation::StreamTransformSink;
use indexmap::IndexMap;
use rstest::rstest;

use super::process_streaming_transform;
use crate::guardrails::chat_guardrail_translation::streaming::tests::support::{
    FakeGuardrail, TWO_CHOICES, chunks, principal,
};

async fn run(
    stream: &[Object],
    guardrail: &FakeGuardrail,
    sink: &mut StreamTransformSink,
) -> Result<(), GuardrailFailure> {
    process_streaming_transform(stream, guardrail, &principal(), sink).await
}

fn reporting(texts: Option<Vec<&'static str>>, holdback: Option<Vec<i64>>) -> FakeGuardrail {
    FakeGuardrail::answering(Box::new(move |inputs| GuardrailApiInputs {
        texts: texts
            .clone()
            .map(|items| items.into_iter().map(str::to_owned).collect()),
        stream_holdback_chars: holdback.clone(),
        ..inputs
    }))
}

#[tokio::test]
async fn streaming_transform_reports_the_guardrailed_text_per_choice_and_keeps_the_chunks() {
    let guardrail = FakeGuardrail::upper_casing();
    let stream = chunks(TWO_CHOICES);
    let mut sink = StreamTransformSink::default();
    run(&stream, &guardrail, &mut sink).await.unwrap();
    let expected = IndexMap::from([(0_i64, "HELLO".to_owned()), (1, "HI".to_owned())]);
    assert_eq!(sink.mutated_text_per_choice, expected);
    assert!(sink.holdback_per_choice.is_empty());
    assert_eq!(stream, chunks(TWO_CHOICES));
}

#[tokio::test]
async fn streaming_transform_hands_the_guardrail_the_texts_in_choice_index_order() {
    let guardrail = reporting(Some(vec!["zero!", "one!"]), None);
    let stream = chunks(
        r#"[{"choices": [{"index": 1, "delta": {"content": "one"}}]}, {"choices": [{"index": 0, "delta": {"content": "zero"}}]}]"#,
    );
    let mut sink = StreamTransformSink::default();
    run(&stream, &guardrail, &mut sink).await.unwrap();
    assert_eq!(
        guardrail.calls()[0].inputs.texts,
        Some(vec!["zero".to_owned(), "one".to_owned()])
    );
    let reported: Vec<(i64, String)> = sink.mutated_text_per_choice.into_iter().collect();
    assert_eq!(reported, [(0, "zero!".to_owned()), (1, "one!".to_owned())]);
}

#[tokio::test]
async fn streaming_transform_hands_the_guardrail_the_chunks_model_and_key_metadata() {
    let guardrail = FakeGuardrail::passing_through();
    let stream = chunks(TWO_CHOICES);
    let mut sink = StreamTransformSink::default();
    run(&stream, &guardrail, &mut sink).await.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    assert_eq!(calls[0].inputs.model.as_deref(), Some("gpt-4o"));
    assert!(calls[0].request_data.contains_key("responses"));
    assert!(calls[0].request_data.contains_key("metadata"));
}

#[tokio::test]
async fn streaming_transform_takes_the_raw_text_as_guardrailed_when_no_texts_come_back() {
    let guardrail = reporting(None, None);
    let stream = chunks(TWO_CHOICES);
    let mut sink = StreamTransformSink::default();
    run(&stream, &guardrail, &mut sink).await.unwrap();
    let expected = IndexMap::from([(0_i64, "Hello".to_owned()), (1, "Hi".to_owned())]);
    assert_eq!(sink.mutated_text_per_choice, expected);
}

#[rstest]
#[case::fewer_texts(vec!["X"], vec![(0_i64, "X")])]
#[case::more_texts(vec!["X", "Y", "Z"], vec![(0_i64, "X"), (1, "Y")])]
#[case::no_texts(vec![], vec![])]
#[tokio::test]
async fn streaming_transform_pairs_returned_texts_with_choices_in_order(
    #[case] returned: Vec<&'static str>,
    #[case] expected: Vec<(i64, &str)>,
) {
    let guardrail = reporting(Some(returned), None);
    let stream = chunks(TWO_CHOICES);
    let mut sink = StreamTransformSink::default();
    run(&stream, &guardrail, &mut sink).await.unwrap();
    let reported: Vec<(i64, String)> = sink.mutated_text_per_choice.into_iter().collect();
    let wanted: Vec<(i64, String)> = expected
        .into_iter()
        .map(|(index, text)| (index, text.to_owned()))
        .collect();
    assert_eq!(reported, wanted);
}

#[rstest]
#[case::both_choices(Some(vec![3, 5]), vec![(0_i64, 3_i64), (1, 5)])]
#[case::one_choice_only(Some(vec![4]), vec![(0, 4)])]
#[case::more_than_choices(Some(vec![1, 2, 3]), vec![(0, 1), (1, 2)])]
#[case::negative_count_is_zero(Some(vec![-2, 7]), vec![(0, 0), (1, 7)])]
#[case::too_large_count_is_zero(Some(vec![7_000_000_000, 7]), vec![(0, 0), (1, 7)])]
#[case::none_requested(None, vec![])]
#[tokio::test]
async fn streaming_transform_reports_the_holdback_of_each_choice(
    #[case] holdback: Option<Vec<i64>>,
    #[case] expected: Vec<(i64, i64)>,
) {
    let guardrail = reporting(None, holdback);
    let stream = chunks(TWO_CHOICES);
    let mut sink = StreamTransformSink::default();
    run(&stream, &guardrail, &mut sink).await.unwrap();
    let reported: Vec<(i64, i64)> = sink.holdback_per_choice.into_iter().collect();
    assert_eq!(reported, expected);
}

#[rstest]
#[case::no_chunks("[]")]
#[case::role_only(r#"[{"choices": [{"index": 0, "delta": {"role": "assistant"}}]}]"#)]
#[case::empty_text(r#"[{"choices": [{"index": 0, "delta": {"content": ""}}]}]"#)]
#[case::list_content(
    r#"[{"choices": [{"index": 0, "delta": {"content": [{"type": "text", "text": "a"}]}}]}]"#
)]
#[tokio::test]
async fn streaming_transform_clears_the_sink_when_there_is_no_text(#[case] source: &str) {
    let guardrail = FakeGuardrail::passing_through();
    let mut sink = StreamTransformSink {
        mutated_text_per_choice: IndexMap::from([(0_i64, "stale".to_owned())]),
        holdback_per_choice: IndexMap::from([(0_i64, 9_i64)]),
    };
    run(&chunks(source), &guardrail, &mut sink).await.unwrap();
    assert!(sink.mutated_text_per_choice.is_empty());
    assert!(sink.holdback_per_choice.is_empty());
    assert!(guardrail.calls().is_empty());
}

#[tokio::test]
async fn streaming_transform_passes_on_the_guardrail_failure() {
    let blocked = GuardrailFailure::Http {
        status: 400,
        detail: "blocked".to_owned(),
    };
    let guardrail = FakeGuardrail::failing(blocked.clone());
    let mut sink = StreamTransformSink::default();
    let failure = run(&chunks(TWO_CHOICES), &guardrail, &mut sink)
        .await
        .unwrap_err();
    assert_eq!(failure, blocked);
}
