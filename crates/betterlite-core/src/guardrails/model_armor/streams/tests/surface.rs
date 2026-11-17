use rstest::rstest;

use crate::guardrails::model_armor::streams::chunk::Chunk;
use crate::guardrails::model_armor::streams::surface::{Surface, classify, is_terminal_error};

use super::support::{FakeWire, Setup, WireFlags, bytes, chunk, fixture};

fn surface_of(label: &str) -> Surface {
    match label {
        "anthropic" => Surface::AnthropicMessages,
        "responses" => Surface::Responses,
        "opaque" => Surface::OpaqueSse,
        _ => Surface::ChatCompletions,
    }
}

#[rstest]
#[case::chat_completion(r#"{"id": "chatcmpl-1", "choices": []}"#, "chat")]
#[case::responses_event(r#"{"type": "response.created", "response": {}}"#, "responses")]
#[case::responses_delta(r#"{"type": "response.output_text.delta", "delta": "x"}"#, "responses")]
#[case::null_type(r#"{"type": null, "choices": []}"#, "chat")]
#[case::non_text_type(r#"{"type": 7, "choices": []}"#, "chat")]
fn classify_json_chunks_into_their_wire_surface(#[case] json: &str, #[case] expected: &str) {
    let wire = FakeWire::new(Setup::default());
    let chunks = [chunk(json)];
    assert_eq!(classify(&wire, &chunks), surface_of(expected));
}

#[test]
fn classify_raw_frames_as_anthropic_when_the_wire_recognizes_them() {
    let mut setup = fixture();
    setup.wire = WireFlags {
        anthropic: true,
        sse_error: false,
    };
    let wire = FakeWire::new(setup);
    let chunks = [bytes("event: message_start\ndata: {}\n\n")];
    assert_eq!(classify(&wire, &chunks), Surface::AnthropicMessages);
}

#[test]
fn classify_raw_frames_as_opaque_sse_when_the_wire_does_not_recognize_them() {
    let wire = FakeWire::new(Setup::default());
    let chunks = [bytes("event: generateContent\ndata: {}\n\n")];
    assert_eq!(classify(&wire, &chunks), Surface::OpaqueSse);
}

#[test]
fn classify_mixed_frames_by_the_raw_bytes_they_carry() {
    let mut setup = fixture();
    setup.wire = WireFlags {
        anthropic: true,
        sse_error: false,
    };
    let wire = FakeWire::new(setup);
    let chunks = [
        bytes("event: message_start\ndata: {}\n\n"),
        chunk(r#"{"type": "response.created"}"#),
    ];
    assert_eq!(classify(&wire, &chunks), Surface::AnthropicMessages);
}

#[test]
fn terminal_error_when_every_chunk_carries_the_error_type() {
    let wire = FakeWire::new(Setup::default());
    let chunks = [
        chunk(r#"{"type": "error", "error": {}}"#),
        chunk(r#"{"type": "error"}"#),
    ];
    assert!(is_terminal_error(&wire, &chunks));
}

#[test]
fn terminal_error_is_false_when_a_chunk_carries_a_message() {
    let wire = FakeWire::new(Setup::default());
    let chunks = [
        chunk(r#"{"type": "error"}"#),
        chunk(r#"{"id": "chatcmpl-1"}"#),
    ];
    assert!(!is_terminal_error(&wire, &chunks));
}

#[test]
fn terminal_error_is_false_for_chat_chunks() {
    let wire = FakeWire::new(Setup::default());
    let chunks = [chunk(r#"{"choices": []}"#)];
    assert!(!is_terminal_error(&wire, &chunks));
}

#[test]
fn terminal_error_when_the_wire_reports_an_sse_error_stream() {
    let mut setup = fixture();
    setup.wire = WireFlags {
        anthropic: false,
        sse_error: true,
    };
    let wire = FakeWire::new(setup);
    let chunks = [bytes("data: {\"error\": {\"message\": \"blocked\"}}\n\n")];
    assert!(is_terminal_error(&wire, &chunks));
}

#[test]
fn terminal_error_matches_an_empty_stream_before_the_hook_checks_emptiness() {
    let wire = FakeWire::new(Setup::default());
    let chunks: [Chunk; 0] = [];
    assert!(is_terminal_error(&wire, &chunks));
}
