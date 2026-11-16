#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_message_scope::Object;

use super::process_streaming_block_only;
use crate::guardrails::chat_guardrail_translation::rewrites::error::EndedStreamError;
use crate::guardrails::chat_guardrail_translation::streaming::tests::support::{
    ConcatenatingAssembler, ENDED, FakeGuardrail, TWO_CHOICES, chunks, ended, object, principal,
};

async fn run(
    stream: &mut [Object],
    guardrail: &FakeGuardrail,
    deliver_rewrites: bool,
) -> Result<(), EndedStreamError> {
    let principal = principal();
    let context = ended(&principal, deliver_rewrites);
    process_streaming_block_only(stream, guardrail, &ConcatenatingAssembler, &context).await
}

#[tokio::test]
async fn block_only_scans_the_text_so_far_of_every_choice() {
    let guardrail = FakeGuardrail::passing_through();
    let mut stream = chunks(TWO_CHOICES);
    run(&mut stream, &guardrail, false).await.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    assert_eq!(
        calls[0].inputs.texts,
        Some(vec!["Hello".to_owned(), "Hi".to_owned()])
    );
    assert_eq!(calls[0].inputs.model.as_deref(), Some("gpt-4o"));
}

#[tokio::test]
async fn block_only_hands_the_guardrail_the_chunks_and_the_key_metadata() {
    let guardrail = FakeGuardrail::passing_through();
    let mut stream = chunks(TWO_CHOICES);
    run(&mut stream, &guardrail, false).await.unwrap();
    let request_data = &guardrail.calls()[0].request_data;
    let Some(GuardrailValue::Array(sent)) = request_data.get("responses") else {
        panic!("responses missing");
    };
    assert_eq!(sent.len(), 3);
    let expected = GuardrailValue::Object(object(
        r#"{"user_api_key_user_id": "user-1", "user_api_key_user_email": "user@example.com"}"#,
    ));
    assert_eq!(request_data.get("metadata"), Some(&expected));
}

#[tokio::test]
async fn block_only_writes_the_rewrite_into_the_first_chunk_and_blanks_the_rest() {
    let guardrail = FakeGuardrail::upper_casing();
    let mut stream = chunks(TWO_CHOICES);
    run(&mut stream, &guardrail, false).await.unwrap();
    let expected = chunks(
        r#"[
        {"model": "gpt-4o", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "HELLO"}}]},
        {"choices": [{"index": 1, "delta": {"content": "HI"}}]},
        {"choices": [{"index": 0, "delta": {"content": ""}}]}]"#,
    );
    assert_eq!(stream, expected);
}

#[tokio::test]
async fn block_only_rewrites_the_text_of_list_content_per_item() {
    let guardrail = FakeGuardrail::upper_casing();
    let mut stream = chunks(
        r#"[{"choices": [{"index": 0, "delta": {"content": [{"type": "text", "text": "ab"}]}}]}]"#,
    );
    run(&mut stream, &guardrail, false).await.unwrap();
    let expected = chunks(
        r#"[{"choices": [{"index": 0, "delta": {"content": [{"type": "text", "text": "AB"}]}}]}]"#,
    );
    assert_eq!(stream, expected);
}

#[tokio::test]
async fn block_only_leaves_the_stream_alone_when_the_guardrail_returns_no_texts() {
    let guardrail = FakeGuardrail::answering(Box::new(|inputs| GuardrailApiInputs {
        texts: None,
        ..inputs
    }));
    let mut stream = chunks(TWO_CHOICES);
    run(&mut stream, &guardrail, false).await.unwrap();
    assert_eq!(stream, chunks(TWO_CHOICES));
}

#[tokio::test]
async fn block_only_fails_when_the_guardrail_returns_more_texts_than_it_was_sent() {
    let guardrail = FakeGuardrail::answering(Box::new(|inputs| GuardrailApiInputs {
        texts: Some(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]),
        ..inputs
    }));
    let mut stream = chunks(TWO_CHOICES);
    let failure = run(&mut stream, &guardrail, false).await.unwrap_err();
    assert!(matches!(failure, EndedStreamError::Rewrite(_)));
}

#[tokio::test]
async fn block_only_passes_on_the_guardrail_failure() {
    let blocked = GuardrailFailure::Http {
        status: 400,
        detail: "blocked".to_owned(),
    };
    let guardrail = FakeGuardrail::failing(blocked.clone());
    let mut stream = chunks(TWO_CHOICES);
    let failure = run(&mut stream, &guardrail, false).await.unwrap_err();
    assert!(matches!(failure, EndedStreamError::Guardrail(found) if found == blocked));
}

#[tokio::test]
async fn block_only_only_names_the_model_of_the_first_chunk() {
    let guardrail = FakeGuardrail::passing_through();
    let mut stream = chunks(
        r#"[{"choices": [{"index": 0, "delta": {"content": "a"}}]}, {"model": "gpt-4o", "choices": [{"index": 0, "delta": {"content": "b"}}]}]"#,
    );
    run(&mut stream, &guardrail, false).await.unwrap();
    assert_eq!(guardrail.calls()[0].inputs.model, None);
}

#[tokio::test]
async fn block_only_skips_a_stream_without_text() {
    let guardrail = FakeGuardrail::passing_through();
    let mut stream = chunks(r#"[{"choices": [{"index": 0, "delta": {"role": "assistant"}}]}]"#);
    run(&mut stream, &guardrail, false).await.unwrap();
    assert!(guardrail.calls().is_empty());
}

#[tokio::test]
async fn block_only_skips_a_stream_of_tool_call_deltas() {
    let guardrail = FakeGuardrail::passing_through();
    let mut stream = chunks(
        r#"[{"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "a", "function": {"name": "f"}}]}}]}]"#,
    );
    run(&mut stream, &guardrail, false).await.unwrap();
    assert!(guardrail.calls().is_empty());
}

#[tokio::test]
async fn block_only_does_not_treat_a_later_choice_finishing_as_the_end() {
    let guardrail = FakeGuardrail::upper_casing();
    let mut stream = chunks(
        r#"[{"choices": [{"index": 0, "delta": {"content": "a"}}, {"index": 1, "delta": {}, "finish_reason": "stop"}]}]"#,
    );
    run(&mut stream, &guardrail, false).await.unwrap();
    let expected = chunks(
        r#"[{"choices": [{"index": 0, "delta": {"content": "A"}}, {"index": 1, "delta": {}, "finish_reason": "stop"}]}]"#,
    );
    assert_eq!(stream, expected);
}

#[tokio::test]
async fn block_only_rebuilds_an_ended_stream_and_scans_it_whole() {
    let guardrail = FakeGuardrail::upper_casing();
    let mut stream = chunks(ENDED);
    run(&mut stream, &guardrail, true).await.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].inputs.texts, Some(vec!["Hello".to_owned()]));
    let expected = chunks(
        r#"[
        {"id": "c1", "model": "gpt-4o", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "HELLO"}}]},
        {"id": "c1", "model": "gpt-4o", "choices": [{"index": 0, "delta": {"content": ""}}]},
        {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}]"#,
    );
    assert_eq!(stream, expected);
}

#[tokio::test]
async fn block_only_keeps_an_ended_stream_as_it_was_unless_rewrites_are_delivered() {
    let guardrail = FakeGuardrail::upper_casing();
    let mut stream = chunks(ENDED);
    run(&mut stream, &guardrail, false).await.unwrap();
    assert_eq!(guardrail.calls().len(), 1);
    assert_eq!(stream, chunks(ENDED));
}
