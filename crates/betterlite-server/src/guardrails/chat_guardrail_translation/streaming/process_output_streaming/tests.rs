#![cfg(test)]

use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_translation::StreamTransformSink;
use indexmap::IndexMap;

use super::process_output_streaming_response;
use crate::guardrails::chat_guardrail_translation::rewrites::error::EndedStreamError;
use crate::guardrails::chat_guardrail_translation::streaming::tests::support::{
    ConcatenatingAssembler, ENDED, FakeGuardrail, TWO_CHOICES, chunks, ended, principal,
};

async fn run(
    stream: &mut [Object],
    guardrail: &FakeGuardrail,
    sink: Option<&mut StreamTransformSink>,
) -> Result<(), EndedStreamError> {
    let principal = principal();
    let context = ended(&principal, true);
    process_output_streaming_response(stream, guardrail, &ConcatenatingAssembler, &context, sink)
        .await
}

#[tokio::test]
async fn process_output_streaming_with_a_sink_reports_the_text_and_keeps_the_chunks() {
    let guardrail = FakeGuardrail::upper_casing();
    let mut stream = chunks(TWO_CHOICES);
    let mut sink = StreamTransformSink::default();
    run(&mut stream, &guardrail, Some(&mut sink)).await.unwrap();
    let expected = IndexMap::from([(0_i64, "HELLO".to_owned()), (1, "HI".to_owned())]);
    assert_eq!(sink.mutated_text_per_choice, expected);
    assert_eq!(stream, chunks(TWO_CHOICES));
}

#[tokio::test]
async fn process_output_streaming_with_a_sink_scans_an_ended_stream_as_text_so_far() {
    let guardrail = FakeGuardrail::upper_casing();
    let mut stream = chunks(ENDED);
    let mut sink = StreamTransformSink::default();
    run(&mut stream, &guardrail, Some(&mut sink)).await.unwrap();
    assert_eq!(
        sink.mutated_text_per_choice,
        IndexMap::from([(0_i64, "HELLO".to_owned())])
    );
    assert_eq!(stream, chunks(ENDED));
}

#[tokio::test]
async fn process_output_streaming_without_a_sink_runs_the_block_only_path() {
    let guardrail = FakeGuardrail::upper_casing();
    let mut stream = chunks(TWO_CHOICES);
    run(&mut stream, &guardrail, None).await.unwrap();
    assert_ne!(stream, chunks(TWO_CHOICES));
}

#[tokio::test]
async fn process_output_streaming_passes_on_a_transform_failure() {
    let blocked = GuardrailFailure::Http {
        status: 400,
        detail: "blocked".to_owned(),
    };
    let guardrail = FakeGuardrail::failing(blocked.clone());
    let mut stream = chunks(TWO_CHOICES);
    let mut sink = StreamTransformSink::default();
    let failure = run(&mut stream, &guardrail, Some(&mut sink))
        .await
        .unwrap_err();
    assert!(matches!(failure, EndedStreamError::Guardrail(found) if found == blocked));
}
