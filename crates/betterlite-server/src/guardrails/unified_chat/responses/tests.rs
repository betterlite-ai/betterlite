#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};
use rstest::rstest;

use super::{ResponsesTranslation, ResponsesTranslationError};
use crate::guardrails::responses_translation::output_rewrite::error::OutputRewriteError;
use crate::guardrails::unified_chat::tests::support::{
    FakeGuardrail, FixedStamp, cache, context, items, nested, object, other_stamp, stamp, text_at,
    texts_reply, value,
};

type Responses = ResponsesTranslation<FakeGuardrail, FixedStamp>;

const REQUEST: &str = r#"{"model":"gpt-4o","instructions":"be brief","input":[{"type":"message","role":"user","content":"hi"}]}"#;

const RESPONSE: &str = r#"{"id":"resp_1","object":"response","model":"gpt-4o","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}]}"#;

const DELTA: &str =
    r#"[{"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"one"}]"#;

const ROUND_TRIP: &str = r#"["raw",{"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"one"}]"#;

const UNPLACEABLE: &str = r#"[{"type":"response.output_text.delta","delta":"one"}]"#;

fn translation() -> Responses {
    ResponsesTranslation::new(stamp(), ApiLimits::default(), Arc::new(cache()))
}

fn call(deliver: bool) -> TranslationCall<'static, GuardrailLogTarget> {
    TranslationCall {
        user_api_key_dict: None,
        request_data: None,
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: deliver,
    }
}

fn blocked(message: &str) -> OutputRewriteError {
    OutputRewriteError::Guardrail {
        source: Box::new(GuardrailFailure::ModifyResponse(Box::new(
            ModifyResponseFailure {
                message: message.to_owned(),
                model: String::from("gpt-4o"),
                request_data: BTreeMap::new(),
                guardrail_name: None,
                detection_info: BTreeMap::new(),
                original_response: None,
            },
        ))),
    }
}

fn input_content(data: &Object) -> String {
    let Some(GuardrailValue::Array(items)) = data.get("input") else {
        panic!("no input array");
    };
    text_at(&items[0], &["content"])
}

fn output_text(response: &GuardrailValue) -> String {
    let Some(GuardrailValue::Array(output)) = nested(response, &["output"]) else {
        panic!("no output array");
    };
    let Some(GuardrailValue::Array(content)) = nested(&output[0], &["content"]) else {
        panic!("no content array");
    };
    text_at(&content[0], &["text"])
}

#[tokio::test]
async fn adapter_responses_input_rewrites_text_and_honours_skip_flags() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let mut applied = context(&fake, Some("guard"));
    applied.skip_flags.skip_system_message = Some(true);
    let mut data = object(REQUEST);
    translation()
        .process_input_messages(&mut data, &applied, None)
        .await
        .unwrap();
    assert_eq!(input_content(&data), "masked");
    assert_eq!(
        data.get("instructions"),
        Some(&GuardrailValue::Text(String::from("be brief")))
    );
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Request);
    assert_eq!(
        calls[0].inputs.texts.as_ref().unwrap(),
        &[String::from("hi")]
    );
}

#[tokio::test]
async fn adapter_responses_output_rewrites_text() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let translated = translation()
        .process_output_response(value(RESPONSE), &applied, call(false))
        .await
        .unwrap();
    assert_eq!(output_text(&translated), "masked");
    assert_eq!(
        nested(&translated, &["model"]),
        Some(&GuardrailValue::Text(String::from("gpt-4o")))
    );
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
}

#[test]
fn adapter_responses_structured_messages_read_the_input() {
    let messages = translation()
        .get_structured_messages(&object(REQUEST))
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[1].role, "user");
}

#[test]
fn adapter_responses_structured_messages_absent_is_none() {
    let messages = translation().get_structured_messages(&object(r#"{"model":"gpt-4o"}"#));
    assert!(messages.is_none());
}

#[test]
fn adapter_responses_tool_names_keep_the_trait_default() {
    let data = object(r#"{"tools":[{"type":"function","name":"lookup"}]}"#);
    assert!(translation().extract_request_tool_names(&data).is_empty());
}

#[test]
fn flags_responses_match_pinned_handler() {
    assert!(Responses::DELIVERS_ENDED_STREAM_REWRITES);
    assert!(Responses::ASSEMBLES_STREAMED_RESPONSE);
}

#[rstest]
#[case::delivery_off(false, "one")]
#[case::delivery_on(true, "masked")]
#[tokio::test]
async fn flags_responses_delivery_flag_reaches_handler(
    #[case] deliver: bool,
    #[case] expected: &str,
) {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let mut stream = items(DELTA);
    translation()
        .process_streaming_response(&mut stream, &applied, call(deliver))
        .await
        .unwrap();
    assert_eq!(text_at(&stream[0], &["delta"]), expected);
}

#[tokio::test]
async fn flags_responses_undeliverable_rewrite_keeps_the_name() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let mut stream = items(UNPLACEABLE);
    let error = translation()
        .process_streaming_response(&mut stream, &applied, call(true))
        .await
        .unwrap_err();
    match error {
        ResponsesTranslationError::Undeliverable(rewrite) => {
            assert_eq!(rewrite.guardrail_name, "guard");
        }
        other => panic!("expected the undeliverable rewrite, got {other:?}"),
    }
}

#[tokio::test]
async fn stream_responses_events_edit_in_place() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let mut stream = items(ROUND_TRIP);
    translation()
        .process_streaming_response(&mut stream, &applied, call(true))
        .await
        .unwrap();
    assert_eq!(stream.len(), 2);
    assert_eq!(stream[0], GuardrailValue::Text(String::from("raw")));
    assert_eq!(text_at(&stream[1], &["delta"]), "masked");
}

#[test]
fn stream_responses_scan_key_follows_the_events() {
    let stream = items(DELTA);
    let key = translation().get_streaming_scan_key(&stream).unwrap();
    assert_eq!(key.texts, vec![String::from("one")]);
    assert!(!key.stream_ended);
}

#[test]
fn stamp_responses_block_chunks_use_the_injected_stamp() {
    let failure = ResponsesTranslationError::Output(blocked("blocked"));
    let frames = translation()
        .build_block_sse_chunks(&failure, false, None)
        .unwrap();
    let text = String::from_utf8(frames.concat()).unwrap();
    assert!(text.contains("resp_fixed"));
    assert!(text.contains("msg_fixed"));
    assert!(text.contains("\"created_at\":1234"));
    assert!(text.contains("blocked"));
    let other: Responses =
        ResponsesTranslation::new(other_stamp(), ApiLimits::default(), Arc::new(cache()));
    let other_frames = other.build_block_sse_chunks(&failure, false, None).unwrap();
    let other_text = String::from_utf8(other_frames.concat()).unwrap();
    assert!(other_text.contains("resp_other"));
    assert!(other_text.contains("\"created_at\":99"));
    assert_ne!(text, other_text);
}

#[test]
fn stamp_responses_error_items_use_the_failure_status() {
    let failure = ResponsesTranslationError::Output(blocked("blocked"));
    let items = translation()
        .build_stream_error_items(&failure, None)
        .unwrap();
    let [GuardrailValue::Object(_)] = items.as_slice() else {
        panic!("expected one error item, got {items:?}");
    };
    assert_eq!(text_at(&items[0], &["error", "code"]), "400");
    assert_eq!(text_at(&items[0], &["error", "message"]), "blocked");
    assert_eq!(text_at(&items[0], &["error", "type"]), "guardrail_error");
}
