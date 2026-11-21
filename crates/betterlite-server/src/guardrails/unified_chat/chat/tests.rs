#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};

use super::support::write_back;
use super::{ChatTranslation, ChatTranslationError};
use crate::guardrails::chat_guardrail_translation::rewrites::error::{
    EndedStreamError, InputError,
};
use crate::guardrails::unified_chat::tests::support::{
    FakeGuardrail, FixedStamp, ScriptedAssembler, chat_response, context, items, nested, object,
    other_stamp, stamp, text_at, texts_reply, value,
};

type Chat = ChatTranslation<FakeGuardrail, FixedStamp, ScriptedAssembler>;

const ASSEMBLED: &str = r#"{"model":"gpt-4o","choices":[{"index":0,"message":{"content":"hel"}}]}"#;

const REQUEST: &str = r#"{"model":"gpt-4o","messages":[{"role":"system","content":"sys"},{"role":"user","content":"hi"}]}"#;

const COMPLETION: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":7,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"vendor_extra":{"keep":true}}"#;

const PARTS: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":7,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":[{"type":"text","text":"hel"},{"type":"text","text":"lo"},{"type":"image_url"}],"tool_calls":[{"id":"call_0","type":"function","function":{"name":"lookup","arguments":"{}"}},{"id":"call_1","type":"function","function":{}}]},"finish_reason":"stop"}],"vendor_extra":{"keep":true}}"#;

const PARTS_AFTER: &str = r#"{"model":"gpt-4o","choices":[{"index":0,"message":{"content":[{"type":"text","text":"masked"},{"type":"text","text":"lo"},{"type":"image_url"}],"tool_calls":[{"id":"call_0","type":"function","function":{"name":"search","arguments":"{\"q\":1}"}},{"id":"call_1","type":"function","function":{}}]}}]}"#;

const RUNNING: &str = r#"[{"id":"chatcmpl-1","object":"chat.completion.chunk","created":7,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"hel"},"finish_reason":null}]},"raw"]"#;

const FINISHED: &str = r#"[{"id":"chatcmpl-1","object":"chat.completion.chunk","created":7,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"hel"},"finish_reason":"stop"}]}]"#;

fn translation() -> Chat {
    ChatTranslation::new(stamp(), ScriptedAssembler(chat_response(ASSEMBLED)))
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
        model: String::from("gpt-4o"),
        request_data: BTreeMap::new(),
        guardrail_name: None,
        detection_info: BTreeMap::new(),
        original_response: None,
    }))
}

fn message_content(data: &Object, index: usize) -> String {
    let Some(GuardrailValue::Array(messages)) = data.get("messages") else {
        panic!("no messages array");
    };
    text_at(&messages[index], &["content"])
}

fn choice_content(response: &GuardrailValue, index: usize) -> String {
    let Some(GuardrailValue::Array(choices)) = nested(response, &["choices"]) else {
        panic!("no choices array");
    };
    text_at(&choices[index], &["message", "content"])
}

fn delta_content(item: &GuardrailValue) -> String {
    let Some(GuardrailValue::Array(choices)) = nested(item, &["choices"]) else {
        panic!("no choices array");
    };
    text_at(&choices[0], &["delta", "content"])
}

fn assert_error_frame(items: &[GuardrailValue], code: &str, message: &str) {
    let [GuardrailValue::Text(frame)] = items else {
        panic!("expected one error frame, got {items:?}");
    };
    assert!(frame.contains(&format!("\"code\":\"{code}\"")));
    assert!(frame.contains(&format!("\"message\":\"{message}\"")));
}

#[tokio::test]
async fn adapter_chat_input_rewrites_text_and_honours_skip_flags() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let mut applied = context(&fake, Some("guard"));
    applied.skip_flags.skip_system_message = Some(true);
    let mut data = object(REQUEST);
    translation()
        .process_input_messages(&mut data, &applied, None)
        .await
        .unwrap();
    assert_eq!(message_content(&data, 0), "sys");
    assert_eq!(message_content(&data, 1), "masked");
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Request);
    assert_eq!(
        calls[0].inputs.texts.as_ref().unwrap(),
        &[String::from("hi")]
    );
    let structured = calls[0].inputs.structured_messages.as_ref().unwrap();
    assert_eq!(structured.len(), 1);
    assert_eq!(structured[0].role, "user");
}

#[tokio::test]
async fn adapter_chat_output_rewrites_text_and_keeps_unknown_fields() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let translated = translation()
        .process_output_response(value(COMPLETION), &applied, call(false))
        .await
        .unwrap();
    assert_eq!(choice_content(&translated, 0), "masked");
    assert_eq!(
        nested(&translated, &["vendor_extra", "keep"]),
        Some(&GuardrailValue::Boolean(true))
    );
    let Some(GuardrailValue::Array(choices)) = nested(&translated, &["choices"]) else {
        panic!("no choices array");
    };
    assert_eq!(
        nested(&choices[0], &["finish_reason"]),
        Some(&GuardrailValue::Text(String::from("stop")))
    );
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
}

#[test]
fn adapter_chat_write_back_rewrites_text_parts_and_tool_calls() {
    let mut response = value(PARTS);
    write_back(
        &mut response,
        &chat_response(PARTS),
        &chat_response(PARTS_AFTER),
    );
    let Some(GuardrailValue::Array(choices)) = nested(&response, &["choices"]) else {
        panic!("no choices array");
    };
    let Some(GuardrailValue::Array(parts)) = nested(&choices[0], &["message", "content"]) else {
        panic!("no content parts");
    };
    assert_eq!(text_at(&parts[0], &["text"]), "masked");
    assert_eq!(text_at(&parts[1], &["text"]), "lo");
    let Some(GuardrailValue::Array(calls)) = nested(&choices[0], &["message", "tool_calls"]) else {
        panic!("no tool calls");
    };
    assert_eq!(text_at(&calls[0], &["function", "name"]), "search");
    assert_eq!(text_at(&calls[0], &["function", "arguments"]), r#"{"q":1}"#);
    assert_eq!(text_at(&calls[1], &["id"]), "call_1");
    assert_eq!(
        nested(&response, &["vendor_extra", "keep"]),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[test]
fn adapter_chat_structured_messages_are_the_request_messages() {
    let messages = translation()
        .get_structured_messages(&object(REQUEST))
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].role, "user");
}

#[test]
fn adapter_chat_structured_messages_absent_is_none() {
    let messages = translation().get_structured_messages(&object(r#"{"model":"gpt-4o"}"#));
    assert!(messages.is_none());
}

#[test]
fn adapter_chat_tool_names_come_from_tools_and_functions() {
    let data = object(
        r#"{"tools":[{"type":"function","function":{"name":"lookup"}},{"type":"custom","name":"skip"}],"functions":[{"name":"legacy"}]}"#,
    );
    assert_eq!(
        translation().extract_request_tool_names(&data),
        vec![String::from("lookup"), String::from("legacy")]
    );
}

#[test]
fn flags_chat_match_pinned_handler() {
    assert!(Chat::DELIVERS_ENDED_STREAM_REWRITES);
    assert!(Chat::ASSEMBLES_STREAMED_RESPONSE);
}

#[tokio::test]
async fn flags_chat_delivery_flag_reaches_handler() {
    for (deliver, expected) in [(false, "hel"), (true, "masked")] {
        let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
        let applied = context(&fake, Some("guard"));
        let mut stream = items(FINISHED);
        translation()
            .process_streaming_response(&mut stream, &applied, call(deliver))
            .await
            .unwrap();
        assert_eq!(delta_content(&stream[0]), expected);
    }
}

#[tokio::test]
async fn stream_chat_chunks_round_trip() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let mut stream = items(RUNNING);
    translation()
        .process_streaming_response(&mut stream, &applied, call(true))
        .await
        .unwrap();
    assert_eq!(stream.len(), 2);
    assert_eq!(delta_content(&stream[0]), "masked");
    assert_eq!(stream[1], GuardrailValue::Text(String::from("raw")));
}

#[test]
fn stream_chat_scan_key_follows_the_chunk_text() {
    let stream = items(RUNNING);
    let key = translation().get_streaming_scan_key(&stream).unwrap();
    assert_eq!(key.texts, vec![String::from("hel")]);
    assert!(!key.stream_ended);
    assert!(key.tool_calls.is_empty());
}

#[test]
fn stamp_chat_block_chunks_use_the_injected_stamp() {
    let failure = ChatTranslationError::Guardrail(blocked("blocked"));
    let frames = translation()
        .build_block_sse_chunks(&failure, false, None)
        .unwrap();
    let text = String::from_utf8(frames.concat()).unwrap();
    assert!(text.contains("chatcmpl-"));
    assert!(text.contains("\"created\":1234"));
    assert!(text.contains("blocked"));
    let other: Chat =
        ChatTranslation::new(other_stamp(), ScriptedAssembler(chat_response(ASSEMBLED)));
    let other_frames = other.build_block_sse_chunks(&failure, false, None).unwrap();
    let other_text = String::from_utf8(other_frames.concat()).unwrap();
    assert!(other_text.contains("\"created\":99"));
    assert_ne!(text, other_text);
}

#[test]
fn stamp_chat_error_items_carry_the_failure_status_and_message() {
    let http = GuardrailFailure::Http {
        status: 429,
        detail: String::from("quota"),
    };
    let direct = ChatTranslationError::Guardrail(http.clone());
    assert_error_frame(
        &translation()
            .build_stream_error_items(&direct, None)
            .unwrap(),
        "429",
        "quota",
    );
    let wrapped = ChatTranslationError::Stream(EndedStreamError::Guardrail(http));
    assert_error_frame(
        &translation()
            .build_stream_error_items(&wrapped, None)
            .unwrap(),
        "429",
        "quota",
    );
    let input = ChatTranslationError::Input(InputError::Guardrail(GuardrailFailure::Http {
        status: 401,
        detail: String::from("denied"),
    }));
    assert_error_frame(
        &translation()
            .build_stream_error_items(&input, None)
            .unwrap(),
        "401",
        "denied",
    );
}

#[test]
fn stamp_chat_unknown_failures_have_no_frames_or_items() {
    let other = ChatTranslationError::Guardrail(GuardrailFailure::Other(String::from("boom")));
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
    let parse = ChatTranslationError::Parse {
        source: serde_json::from_str::<i32>("x").unwrap_err(),
    };
    assert!(
        translation()
            .build_block_sse_chunks(&parse, false, None)
            .is_none()
    );
}
