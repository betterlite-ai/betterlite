use std::str::from_utf8;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrails::model_armor::streams::chunk::Chunk;
use crate::guardrails::model_armor::streams::errors::{
    HttpError, error_payload, frame_error, sse_frame,
};
use crate::guardrails::model_armor::streams::surface::Surface;

use super::support::{FakeWire, Setup, bytes, chunk, fixture, value};

fn http(status: u16, detail: GuardrailValue) -> HttpError {
    HttpError { status, detail }
}

#[test]
fn error_payload_wraps_a_text_detail_under_message() {
    let payload = error_payload(&http(500, GuardrailValue::Text("boom".to_owned())))
        .expect("the payload encodes");
    assert_eq!(payload, value(r#"{"message": "boom", "code": "500"}"#));
}

#[test]
fn error_payload_unwraps_the_error_field_of_a_mapping_detail() {
    let detail = value(r#"{"error": "blocked", "model_armor_response": {"a": 1}}"#);
    let payload = error_payload(&http(400, detail)).expect("the payload encodes");
    assert_eq!(payload, value(r#"{"message": "blocked", "code": "400"}"#));
}

#[test]
fn error_payload_keeps_a_mapping_error_value_whole() {
    let detail = value(r#"{"error": {"message": "blocked", "reason": "rai"}}"#);
    let payload = error_payload(&http(400, detail)).expect("the payload encodes");
    assert_eq!(
        payload,
        value(r#"{"message": "blocked", "reason": "rai", "code": "400"}"#)
    );
}

#[test]
fn error_payload_wraps_a_non_mapping_error_value_as_a_message() {
    let detail = value(r#"{"error": 7}"#);
    let payload = error_payload(&http(400, detail)).expect("the payload encodes");
    assert_eq!(payload, value(r#"{"message": "7", "code": "400"}"#));
}

#[test]
fn error_payload_overwrites_a_code_the_detail_already_carries() {
    let detail = value(r#"{"message": "blocked", "code": "upstream"}"#);
    let payload = error_payload(&http(400, detail)).expect("the payload encodes");
    assert_eq!(payload, value(r#"{"message": "blocked", "code": "400"}"#));
}

#[test]
fn frame_error_uses_anthropic_frames_on_the_anthropic_surface() {
    let frames = vec![bytes("event: error\ndata: {}\n\n")];
    let mut setup = fixture();
    setup.frames = frames.clone();
    let wire = FakeWire::new(setup);
    let error = http(
        400,
        value(r#"{"error": "Streaming response blocked by Model Armor"}"#),
    );
    let items = frame_error(&wire, Surface::AnthropicMessages, &error).expect("the frames build");
    assert_eq!(items, frames);
    assert_eq!(
        wire.frame_messages(),
        vec!["Streaming response blocked by Model Armor".to_owned()]
    );
}

#[test]
fn error_frame_anthropic_message_is_empty_without_a_message_field() {
    let wire = FakeWire::new(Setup::default());
    let error = http(400, value(r#"{"reason": "rai"}"#));
    let _items = frame_error(&wire, Surface::AnthropicMessages, &error).expect("the frames build");
    assert_eq!(wire.frame_messages(), vec![String::new()]);
}

#[test]
fn frame_error_uses_responses_error_items_when_the_wire_builds_them() {
    let items = vec![chunk(r#"{"type": "error"}"#)];
    let mut setup = fixture();
    setup.responses = Some(items.clone());
    let wire = FakeWire::new(setup);
    let error = http(500, GuardrailValue::Text("boom".to_owned()));
    let framed = frame_error(&wire, Surface::Responses, &error).expect("the frames build");
    assert_eq!(framed, items);
    assert!(wire.frame_messages().is_empty());
}

#[test]
fn frame_error_falls_back_to_sse_for_responses_without_items() {
    let wire = FakeWire::new(Setup::default());
    let error = http(500, GuardrailValue::Text("boom".to_owned()));
    let framed = frame_error(&wire, Surface::Responses, &error).expect("the frames build");
    assert_eq!(
        framed,
        sse_frame(&value(r#"{"message": "boom", "code": "500"}"#)).expect("the frame encodes")
    );
}

#[test]
fn frame_error_falls_back_to_sse_for_an_empty_responses_item_list() {
    let mut setup = fixture();
    setup.responses = Some(Vec::new());
    let wire = FakeWire::new(setup);
    let error = http(500, GuardrailValue::Text("boom".to_owned()));
    let framed = frame_error(&wire, Surface::Responses, &error).expect("the frames build");
    assert_eq!(
        framed,
        sse_frame(&value(r#"{"message": "boom", "code": "500"}"#)).expect("the frame encodes")
    );
}

#[test]
fn frame_error_encodes_sse_for_chat_completions() {
    let wire = FakeWire::new(Setup::default());
    let error = http(400, GuardrailValue::Text("blocked".to_owned()));
    let framed = frame_error(&wire, Surface::ChatCompletions, &error).expect("the frames build");
    assert_eq!(framed.len(), 1);
}

#[test]
fn frame_error_encodes_sse_for_opaque_frames() {
    let wire = FakeWire::new(Setup::default());
    let error = http(400, GuardrailValue::Text("blocked".to_owned()));
    let framed = frame_error(&wire, Surface::OpaqueSse, &error).expect("the frames build");
    assert_eq!(framed.len(), 1);
}

#[test]
fn error_sse_frame_wraps_the_payload_under_error() {
    let framed =
        sse_frame(&value(r#"{"message": "boom", "code": "500"}"#)).expect("the frame encodes");
    let Chunk::Bytes(raw) = &framed[0] else {
        panic!("the fallback frame is raw SSE");
    };
    let text = from_utf8(raw).expect("the frame is utf-8");
    assert_eq!(
        text,
        "data: {\"error\":{\"code\":\"500\",\"message\":\"boom\"}}\n\n"
    );
}
