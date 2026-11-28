#![cfg(test)]

use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::str;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_message_scope::stream_chunk_events::stream_item_value;
use betterlite_guardrails::guardrail_message_scope::{Object, StreamItem};
use betterlite_guardrails::guardrail_translation::{BaseTranslation as _, TranslationCall};
use rstest::rstest;

use super::{AnthropicTranslation, AnthropicTranslationError};
use crate::guardrails::anthropic_translation::process_stream::error::ProcessStreamError;
use crate::guardrails::unified_chat::tests::support::{
    FakeGuardrail, FixedStamp, context, items, nested, object, other_stamp, stamp, text_at,
    texts_reply, value,
};

type Anthropic = AnthropicTranslation<FakeGuardrail, FixedStamp>;

const REQUEST: &str = r#"{"model":"claude-3","max_tokens":10,"system":"be brief","messages":[{"role":"user","content":"hi"}]}"#;

const MESSAGE: &str = r#"{"id":"msg_1","type":"message","role":"assistant","model":"claude-3","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#;

const CHAT_COMPLETION: &str = r#"{"id":"chatcmpl-1","model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;

const DELTA: &str =
    r#"[{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"one"}}]"#;

const ROUND_TRIP: &str = r#"["raw",{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"one"}}]"#;

const NO_TEXT_DELTA: &str =
    r#"[{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}]"#;

const TEXT_DELTA_FRAME: &[u8] = b"event: content_block_delta\ndata: {\"type\": \"content_block_delta\", \"index\": 0, \"delta\": {\"type\": \"text_delta\", \"text\": \"one\"}}\n\n";

const ENDED_FRAME: &[u8] = b"event: content_block_delta\ndata: {\"type\": \"content_block_delta\", \"index\": 0, \"delta\": {\"type\": \"text_delta\", \"text\": \"one\"}}\n\nevent: message_delta\ndata: {\"type\": \"message_delta\", \"delta\": {\"stop_reason\": \"end_turn\"}}\n\n";

fn translation() -> Anthropic {
    AnthropicTranslation {
        stamps: stamp(),
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

fn blocked(message: &str) -> GuardrailFailure {
    GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
        message: message.to_owned(),
        model: String::from("claude-3"),
        request_data: BTreeMap::new(),
        guardrail_name: None,
        detection_info: BTreeMap::new(),
        original_response: None,
    }))
}

fn message_content(data: &Object) -> String {
    let Some(GuardrailValue::Array(messages)) = data.get("messages") else {
        panic!("no messages array");
    };
    text_at(&messages[0], &["content"])
}

fn response_text(response: &GuardrailValue) -> String {
    let Some(GuardrailValue::Array(content)) = nested(response, &["content"]) else {
        panic!("no content array");
    };
    text_at(&content[0], &["text"])
}

#[tokio::test]
async fn adapter_anthropic_input_rewrites_text_and_honours_skip_flags() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let mut applied = context(&fake, Some("guard"));
    applied.skip_flags.skip_system_message = Some(true);
    let mut data = object(REQUEST);
    translation()
        .process_input_messages(&mut data, &applied, None)
        .await
        .unwrap();
    assert_eq!(message_content(&data), "masked");
    assert_eq!(
        data.get("system"),
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
async fn adapter_anthropic_output_rewrites_text() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let translated = translation()
        .process_output_response(value(MESSAGE), &applied, call(false))
        .await
        .unwrap();
    assert_eq!(response_text(&translated), "masked");
    assert_eq!(
        nested(&translated, &["id"]),
        Some(&GuardrailValue::Text(String::from("msg_1")))
    );
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
}

#[test]
fn adapter_anthropic_post_call_hook_translates_a_chat_completion() {
    let translated = translation().post_call_hook_response(value(CHAT_COMPLETION));
    assert_eq!(text_at(&translated, &["type"]), "message");
    assert_eq!(text_at(&translated, &["id"]), "chatcmpl-1");
    assert_eq!(text_at(&translated, &["stop_reason"]), "end_turn");
    assert_eq!(response_text(&translated), "hello");
}

#[test]
fn adapter_anthropic_post_call_hook_leaves_a_message_alone() {
    let message = value(MESSAGE);
    assert_eq!(
        translation().post_call_hook_response(message.clone()),
        message
    );
}

#[test]
fn adapter_anthropic_post_call_hook_keeps_a_response_it_cannot_translate() {
    let unreadable = value(r#"{"choices":[5],"keep":true}"#);
    assert_eq!(
        translation().post_call_hook_response(unreadable.clone()),
        unreadable
    );
}

#[test]
fn adapter_anthropic_structured_messages_keep_the_trait_default() {
    let messages = translation().get_structured_messages(&object(REQUEST));
    assert!(messages.is_none());
}

#[test]
fn adapter_anthropic_tool_names_keep_the_trait_default() {
    let data = object(r#"{"tools":[{"name":"lookup","input_schema":{"type":"object"}}]}"#);
    assert!(translation().extract_request_tool_names(&data).is_empty());
}

#[test]
fn flags_anthropic_match_pinned_handler() {
    let delivers_rewrites = Anthropic::DELIVERS_ENDED_STREAM_REWRITES;
    let assembles_response = Anthropic::ASSEMBLES_STREAMED_RESPONSE;
    assert!(delivers_rewrites);
    assert!(assembles_response);
}

#[rstest]
#[case::delivery_off(false, "one")]
#[case::delivery_on(true, "masked")]
#[tokio::test]
async fn flags_anthropic_delivery_flag_reaches_handler(
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
        text_at(&stream_item_value(&stream[0]), &["delta", "text"]),
        expected
    );
}

#[tokio::test]
async fn flags_anthropic_undeliverable_passes_through_unchanged() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let mut stream = items(NO_TEXT_DELTA);
    let error = translation()
        .process_streaming_response(&mut stream, &applied, call(true))
        .await
        .unwrap_err();
    match error {
        AnthropicTranslationError::Stream(ProcessStreamError::Undeliverable(rewrite)) => {
            assert_eq!(rewrite.guardrail_name, "guard");
        }
        other @ (AnthropicTranslationError::Input(_)
        | AnthropicTranslationError::Output(_)
        | AnthropicTranslationError::Stream(_)) => {
            panic!("expected the undeliverable error unchanged, got {other:?}")
        }
    }
}

#[tokio::test]
async fn stream_anthropic_events_round_trip() {
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
        text_at(&stream_item_value(&stream[1]), &["delta", "text"]),
        "masked"
    );
}

#[tokio::test]
async fn stream_raw_frame_bytes_text_delta_is_scanned() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let mut stream = vec![StreamItem::Bytes(TEXT_DELTA_FRAME.to_vec())];
    translation()
        .process_streaming_response(&mut stream, &applied, call(false))
        .await
        .unwrap();
    assert_eq!(
        fake.calls()[0].inputs.texts.as_ref().unwrap(),
        &[String::from("one")]
    );
    assert_eq!(stream, vec![StreamItem::Bytes(TEXT_DELTA_FRAME.to_vec())]);
}

#[tokio::test]
async fn stream_raw_frame_ended_rewrite_stays_bytes() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let mut stream = vec![StreamItem::Bytes(ENDED_FRAME.to_vec())];
    translation()
        .process_streaming_response(&mut stream, &applied, call(true))
        .await
        .unwrap();
    let [StreamItem::Bytes(frame)] = stream.as_slice() else {
        panic!("expected the raw frame in place, got {stream:?}");
    };
    let text = str::from_utf8(frame).unwrap();
    assert!(text.contains("\"text\": \"masked\""), "{text}");
}

#[tokio::test]
async fn stream_raw_frame_json_text_is_neither_scanned_nor_changed() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let frame = String::from_utf8(TEXT_DELTA_FRAME.to_vec()).unwrap();
    let mut stream = vec![StreamItem::Json(GuardrailValue::Text(frame.clone()))];
    translation()
        .process_streaming_response(&mut stream, &applied, call(false))
        .await
        .unwrap();
    assert_eq!(
        fake.calls()[0].inputs.texts.as_ref().unwrap(),
        &[String::new()]
    );
    assert_eq!(stream, vec![StreamItem::Json(GuardrailValue::Text(frame))]);
}

#[test]
fn stream_anthropic_scan_key_follows_the_deltas() {
    let stream = items(DELTA);
    let key = translation().get_streaming_scan_key(&stream).unwrap();
    assert_eq!(key.texts, vec![String::from("one")]);
    assert!(!key.stream_ended);
}

#[test]
fn stamp_anthropic_block_chunks_use_the_message_id() {
    let failure =
        AnthropicTranslationError::from(ProcessStreamError::Guardrail(blocked("blocked")));
    let frames = translation()
        .build_block_sse_chunks(&failure, false, None)
        .unwrap();
    let text = String::from_utf8(frames.concat()).unwrap();
    assert!(text.contains("msg_fixed"));
    assert!(text.contains("blocked"));
    let other: Anthropic = AnthropicTranslation {
        stamps: other_stamp(),
        _guardrail: PhantomData,
    };
    let other_frames = other.build_block_sse_chunks(&failure, false, None).unwrap();
    let other_text = String::from_utf8(other_frames.concat()).unwrap();
    assert!(other_text.contains("msg_other"));
    assert_ne!(text, other_text);
}

#[test]
fn stream_raw_frame_error_items_are_bytes() {
    let failure =
        AnthropicTranslationError::from(ProcessStreamError::Guardrail(GuardrailFailure::Http {
            status: 429,
            detail: String::from("quota"),
        }));
    let items = translation()
        .build_stream_error_items(&failure, None)
        .unwrap();
    let [StreamItem::Bytes(frame)] = items.as_slice() else {
        panic!("expected one error frame, got {items:?}");
    };
    assert!(str::from_utf8(frame).unwrap().contains("quota"));
}

#[test]
fn stream_anthropic_mid_stream_block_continues_without_a_new_message() {
    let failure =
        AnthropicTranslationError::from(ProcessStreamError::Guardrail(blocked("blocked")));
    let stream = items(
        r#"[{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}]"#,
    );
    let frames = translation()
        .build_block_sse_chunks(&failure, true, Some(&stream))
        .unwrap();
    assert_eq!(frames.len(), 6);
    assert_eq!(
        str::from_utf8(&frames[0]).unwrap(),
        "event: content_block_stop\ndata: {\"type\": \"content_block_stop\", \"index\": 0}\n\n"
    );
    assert_eq!(
        str::from_utf8(&frames[1]).unwrap(),
        "event: content_block_start\ndata: {\"type\": \"content_block_start\", \"index\": 1, \"content_block\": {\"type\": \"text\", \"text\": \"\"}}\n\n"
    );
    assert!(
        str::from_utf8(&frames[2])
            .unwrap()
            .contains("\"text\": \"blocked\"")
    );
    assert!(
        str::from_utf8(&frames[3])
            .unwrap()
            .starts_with("event: content_block_stop\n")
    );
    assert!(
        str::from_utf8(&frames[4])
            .unwrap()
            .starts_with("event: message_delta\n")
    );
    assert_eq!(
        str::from_utf8(&frames[5]).unwrap(),
        "event: message_stop\ndata: {\"type\": \"message_stop\"}\n\n"
    );
    let text = String::from_utf8(frames.concat()).unwrap();
    assert!(!text.contains("message_start"));
}

#[tokio::test]
async fn adapter_anthropic_output_forwards_the_api_key_and_request_data() {
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
            value(MESSAGE),
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
    assert_eq!(response_text(&translated), "masked");
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
async fn stream_anthropic_forwards_the_api_key_and_request_data() {
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
    let mut stream = items(DELTA);
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
    assert_eq!(
        text_at(&stream_item_value(&stream[0]), &["delta", "text"]),
        "masked"
    );
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
fn stamp_anthropic_unknown_failures_have_no_frames_or_items() {
    let other = AnthropicTranslationError::from(ProcessStreamError::Guardrail(
        GuardrailFailure::Other(String::from("boom")),
    ));
    assert!(
        translation()
            .build_block_sse_chunks(&other, false, None)
            .is_none()
    );
    assert!(
        translation()
            .build_stream_error_items(&other, None)
            .is_none()
    );
}
