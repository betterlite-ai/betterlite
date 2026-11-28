#![cfg(test)]

use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_message_scope::stream_chunk_events::stream_item_value;
use betterlite_guardrails::guardrail_message_scope::{Object, StreamItem};
use betterlite_guardrails::guardrail_translation::{BaseTranslation as _, TranslationCall};
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

const TERMINAL: &str = r#"[{"type":"response.output_text.delta","item_id":"m1","output_index":0,"content_index":0,"delta":"one"},{"type":"response.completed","response":{"model":"gpt-4o","output":[{"type":"message","id":"m1","content":[{"type":"output_text","text":"one"}]}]}}]"#;

fn translation() -> Responses {
    ResponsesTranslation {
        stamps: stamp(),
        limits: ApiLimits::default(),
        cache: Arc::new(cache()),
        _guardrail: PhantomData,
    }
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
    let delivers_rewrites = Responses::DELIVERS_ENDED_STREAM_REWRITES;
    let assembles_response = Responses::ASSEMBLES_STREAMED_RESPONSE;
    assert!(delivers_rewrites);
    assert!(assembles_response);
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
    assert_eq!(
        text_at(&stream_item_value(&stream[0]), &["delta"]),
        expected
    );
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
        other @ (ResponsesTranslationError::Request(_)
        | ResponsesTranslationError::Output(_)
        | ResponsesTranslationError::Stream(_)
        | ResponsesTranslationError::Json(_)) => {
            panic!("expected the undeliverable rewrite, got {other:?}")
        }
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
    assert_eq!(
        stream[0],
        StreamItem::Json(GuardrailValue::Text(String::from("raw")))
    );
    assert_eq!(
        text_at(&stream_item_value(&stream[1]), &["delta"]),
        "masked"
    );
}

#[tokio::test]
async fn stream_raw_frame_bytes_slot_keeps_its_position() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let raw = b"event: response.raw\ndata: {\"type\": \"response.raw\"}\n\n".to_vec();
    let mut stream = items(
        r#"[{"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"one"},{"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"two"}]"#,
    );
    stream.insert(1, StreamItem::Bytes(raw.clone()));
    translation()
        .process_streaming_response(&mut stream, &applied, call(true))
        .await
        .unwrap();
    assert_eq!(stream[1], StreamItem::Bytes(raw));
    assert_eq!(
        text_at(&stream_item_value(&stream[0]), &["delta"]),
        "masked"
    );
    assert_eq!(text_at(&stream_item_value(&stream[2]), &["delta"]), "");
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
    let other: Responses = ResponsesTranslation {
        stamps: other_stamp(),
        limits: ApiLimits::default(),
        cache: Arc::new(cache()),
        _guardrail: PhantomData,
    };
    let other_frames = other.build_block_sse_chunks(&failure, false, None).unwrap();
    let other_text = String::from_utf8(other_frames.concat()).unwrap();
    assert!(other_text.contains("resp_other"));
    assert!(other_text.contains("\"created_at\":99"));
    assert_ne!(text, other_text);
}

#[test]
fn stamp_responses_error_items_use_the_failure_status() {
    let failure = ResponsesTranslationError::Output(blocked("blocked"));
    let built = translation()
        .build_stream_error_items(&failure, None)
        .unwrap();
    let [StreamItem::Json(GuardrailValue::Object(_))] = built.as_slice() else {
        panic!("expected one error item, got {built:?}");
    };
    let item = stream_item_value(&built[0]);
    assert_eq!(text_at(&item, &["error", "code"]), "400");
    assert_eq!(text_at(&item, &["error", "message"]), "blocked");
    assert_eq!(text_at(&item, &["error", "type"]), "guardrail_error");
}

#[tokio::test]
async fn adapter_responses_output_forwards_the_api_key_and_request_data() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let user = BTreeMap::from([(
        String::from("user_id"),
        GuardrailValue::Text(String::from("u1")),
    )]);
    let mut request_data = BTreeMap::from([(
        String::from("caller"),
        GuardrailValue::Text(String::from("kept")),
    )]);
    let translated = translation()
        .process_output_response(
            value(RESPONSE),
            &applied,
            TranslationCall {
                user_api_key_dict: Some(&user),
                request_data: Some(&mut request_data),
                logging_obj: None,
                stream_transform_sink: None,
                deliver_ended_stream_rewrites: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(output_text(&translated), "masked");
    let sent = GuardrailValue::Object(fake.calls()[0].request_data.clone());
    assert_eq!(
        nested(&sent, &["litellm_metadata", "user_api_key_user_id"]),
        Some(&GuardrailValue::Text(String::from("u1")))
    );
    assert_eq!(
        nested(&sent, &["caller"]),
        Some(&GuardrailValue::Text(String::from("kept")))
    );
}

#[tokio::test]
async fn stream_responses_forwards_the_api_key_and_request_data() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let user = BTreeMap::from([(
        String::from("user_id"),
        GuardrailValue::Text(String::from("u1")),
    )]);
    let mut request_data = BTreeMap::from([(
        String::from("caller"),
        GuardrailValue::Text(String::from("kept")),
    )]);
    let mut stream = items(TERMINAL);
    translation()
        .process_streaming_response(
            &mut stream,
            &applied,
            TranslationCall {
                user_api_key_dict: Some(&user),
                request_data: Some(&mut request_data),
                logging_obj: None,
                stream_transform_sink: None,
                deliver_ended_stream_rewrites: true,
            },
        )
        .await
        .unwrap();
    let sent = GuardrailValue::Object(fake.calls()[0].request_data.clone());
    assert_eq!(
        nested(&sent, &["litellm_metadata", "user_api_key_user_id"]),
        Some(&GuardrailValue::Text(String::from("u1")))
    );
    assert_eq!(
        nested(&sent, &["caller"]),
        Some(&GuardrailValue::Text(String::from("kept")))
    );
}

#[test]
fn stamp_responses_block_chunks_continue_a_started_stream() {
    let failure = ResponsesTranslationError::Output(blocked("blocked"));
    let stream = items(
        r#"[{"type":"response.created","response":{"id":"resp_stream","model":"gpt-4o"}},{"type":"response.output_text.delta","item_id":"m1","output_index":0,"content_index":0,"delta":"one","sequence_number":41}]"#,
    );
    let frames = translation()
        .build_block_sse_chunks(&failure, true, Some(&stream))
        .unwrap();
    let text = String::from_utf8(frames.concat()).unwrap();
    assert!(!text.contains("response.created"));
    assert!(text.contains("resp_stream"));
    assert!(text.contains("response.completed"));
}

#[test]
fn stamp_responses_error_items_continue_the_stream_sequence() {
    let failure = ResponsesTranslationError::Output(blocked("blocked"));
    let stream = items(
        r#"[{"type":"response.output_text.delta","item_id":"m1","output_index":0,"content_index":0,"delta":"one","sequence_number":41}]"#,
    );
    let built = translation()
        .build_stream_error_items(&failure, Some(&stream))
        .unwrap();
    assert_eq!(
        nested(&stream_item_value(&built[0]), &["sequence_number"]),
        Some(&GuardrailValue::SignedInteger(42))
    );
}
