#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::guardrails::streaming_rewrite::failure::UndeliverableStreamRewrite;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_message_scope::Object;

use super::{EndedStream, process_ended_stream};
use crate::guardrails::chat_guardrail_translation::rewrites::error::{
    EndedStreamError, RewriteError,
};
use crate::guardrails::chat_guardrail_translation::rewrites::tests::assemblers::ConcatenatingAssembler;
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::{
    FakeGuardrail, chunks, object, principal,
};

const TEXT_STREAM: &str = r#"[
    {"choices": [{"index": 0, "delta": {"role": "assistant", "content": "He"}}]},
    {"choices": [{"index": 0, "delta": {"content": "llo"}}]},
    {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}]"#;

const TOOL_STREAM: &str = r#"[
    {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "a", "function": {"name": "lookup", "arguments": ""}}]}}]},
    {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{}"}}]}}]},
    {"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}]"#;

const TWO_CHOICE_TOOL_STREAM: &str = r#"[
    {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "a", "function": {"name": "lookup", "arguments": ""}}]}}]},
    {"choices": [{"index": 1, "delta": {"tool_calls": [{"index": 0, "id": "b", "function": {"name": "lookup", "arguments": ""}}]}}]}]"#;

fn upper_cased(inputs: GuardrailApiInputs) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: inputs
            .texts
            .map(|texts| texts.iter().map(|text| text.to_uppercase()).collect()),
        ..inputs
    }
}

fn renamed(inputs: GuardrailApiInputs) -> GuardrailApiInputs {
    let call = r#"{"function": {"name": "renamed", "arguments": "{\"q\":1}"}}"#;
    GuardrailApiInputs {
        tool_calls: inputs.tool_calls.map(|calls| {
            calls
                .iter()
                .map(|_| GuardrailValue::Object(object(call)))
                .collect()
        }),
        ..inputs
    }
}

async fn process(
    stream: &mut [Object],
    guardrail: &FakeGuardrail,
    assembler: &ConcatenatingAssembler,
    deliver_rewrites: bool,
) -> Result<(), EndedStreamError> {
    let principal = principal();
    let ended = EndedStream {
        principal: &principal,
        guardrail_name: "pii",
        deliver_rewrites,
    };
    process_ended_stream(stream, guardrail, assembler, &ended).await
}

#[tokio::test]
async fn ended_stream_gather_runs_the_response_guardrail_on_the_rebuilt_text() {
    let guardrail = FakeGuardrail::passing_through();
    let mut stream = chunks(TEXT_STREAM);
    let assembler = ConcatenatingAssembler::texts();
    process(&mut stream, &guardrail, &assembler, true)
        .await
        .unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    assert_eq!(calls[0].inputs.texts, Some(vec!["Hello".to_owned()]));
    assert_eq!(stream, chunks(TEXT_STREAM));
}

#[tokio::test]
async fn ended_stream_gather_writes_a_text_rewrite_into_the_first_chunk() {
    let guardrail = FakeGuardrail::answering(Box::new(upper_cased));
    let mut stream = chunks(TEXT_STREAM);
    let assembler = ConcatenatingAssembler::texts();
    process(&mut stream, &guardrail, &assembler, true)
        .await
        .unwrap();
    let expected = TEXT_STREAM
        .replace(r#""content": "He""#, r#""content": "HELLO""#)
        .replace(r#""content": "llo""#, r#""content": """#);
    assert_eq!(stream, chunks(&expected));
}

#[tokio::test]
async fn ended_stream_gather_leaves_the_stream_alone_when_rewrites_are_not_delivered() {
    let guardrail = FakeGuardrail::answering(Box::new(upper_cased));
    let mut stream = chunks(TEXT_STREAM);
    let assembler = ConcatenatingAssembler::texts();
    process(&mut stream, &guardrail, &assembler, false)
        .await
        .unwrap();
    assert_eq!(guardrail.calls().len(), 1);
    assert_eq!(stream, chunks(TEXT_STREAM));
}

#[tokio::test]
async fn ended_stream_gather_writes_a_tool_call_rewrite_across_the_fragments() {
    let guardrail = FakeGuardrail::answering(Box::new(renamed));
    let mut stream = chunks(TOOL_STREAM);
    let assembler = ConcatenatingAssembler::texts_and_a_tool_call();
    process(&mut stream, &guardrail, &assembler, true)
        .await
        .unwrap();
    let expected = TOOL_STREAM
        .replace(
            r#""name": "lookup", "arguments": """#,
            r#""name": "renamed", "arguments": "{\"q\":1}""#,
        )
        .replace(r#"{"arguments": "{}"}"#, r#"{"arguments": ""}"#);
    assert_eq!(stream, chunks(&expected));
}

#[tokio::test]
async fn ended_stream_gather_reports_a_tool_call_rewrite_on_a_multi_choice_stream_as_undeliverable()
{
    let guardrail = FakeGuardrail::answering(Box::new(renamed));
    let mut stream = chunks(TWO_CHOICE_TOOL_STREAM);
    let assembler = ConcatenatingAssembler::texts_and_a_tool_call();
    let failure = process(&mut stream, &guardrail, &assembler, true)
        .await
        .unwrap_err();
    let expected = UndeliverableStreamRewrite {
        guardrail_name: "pii".to_owned(),
        reason: "the stream carries 2 choices and tool-call rewrites are only written back on single-choice streams".to_owned(),
    };
    assert!(matches!(failure, EndedStreamError::Undeliverable(found) if found == expected));
    assert_eq!(stream, chunks(TWO_CHOICE_TOOL_STREAM));
}

#[tokio::test]
async fn ended_stream_gather_passes_a_guardrail_failure_on_and_keeps_the_stream() {
    let blocked = GuardrailFailure::Http {
        status: 400,
        detail: "blocked".to_owned(),
    };
    let guardrail = FakeGuardrail::failing(blocked.clone());
    let mut stream = chunks(TEXT_STREAM);
    let assembler = ConcatenatingAssembler::texts();
    let failure = process(&mut stream, &guardrail, &assembler, true)
        .await
        .unwrap_err();
    assert!(matches!(failure, EndedStreamError::Guardrail(found) if found == blocked));
    assert_eq!(stream, chunks(TEXT_STREAM));
}

#[tokio::test]
async fn ended_stream_gather_refuses_a_stream_it_cannot_rebuild_before_calling_the_guardrail() {
    let guardrail = FakeGuardrail::passing_through();
    let mut stream = chunks(r#"[{"usage": {}}]"#);
    let assembler = ConcatenatingAssembler::texts();
    let failure = process(&mut stream, &guardrail, &assembler, true)
        .await
        .unwrap_err();
    assert!(matches!(
        failure,
        EndedStreamError::Rewrite(RewriteError::NoStreamChoices)
    ));
    assert!(guardrail.calls().is_empty());
}
