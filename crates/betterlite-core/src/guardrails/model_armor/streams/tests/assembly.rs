use crate::guardrails::model_armor::streams::assembly::{Assembly, assemble, terminal_body};
use crate::guardrails::model_armor::streams::surface::Surface;

use super::support::{FakeWire, bytes, chunk, fixture, model_body, value};

#[tokio::test]
async fn assemble_dispatches_chat_completions_to_the_chat_builder() {
    let mut setup = fixture();
    setup.chat = Some(model_body());
    let wire = FakeWire::new(setup);
    let chunks = [chunk(r#"{"choices": []}"#)];
    let assembled = assemble(&wire, Surface::ChatCompletions, &chunks).await;
    assert_eq!(assembled, Some(model_body()));
    assert_eq!(wire.builders(), vec!["chat"]);
}

#[tokio::test]
async fn assemble_dispatches_anthropic_messages_to_the_anthropic_builder() {
    let mut setup = fixture();
    setup.anthropic_body = Some(model_body());
    let wire = FakeWire::new(setup);
    let chunks = [bytes("event: message_start\ndata: {}\n\n")];
    let assembled = assemble(&wire, Surface::AnthropicMessages, &chunks).await;
    assert_eq!(assembled, Some(model_body()));
    assert_eq!(wire.builders(), vec!["anthropic"]);
}

#[tokio::test]
async fn assemble_reads_a_responses_body_from_the_terminal_event() {
    let mut setup = fixture();
    setup.chat = Some(model_body());
    let wire = FakeWire::new(setup);
    let chunks = [
        chunk(r#"{"type": "response.created", "response": {"id": "resp_1", "output": []}}"#),
        chunk(r#"{"type": "response.completed", "response": {"id": "resp_1", "output": []}}"#),
    ];
    let assembled = assemble(&wire, Surface::Responses, &chunks).await;
    assert_eq!(
        assembled,
        Some(Assembly::Responses(value(
            r#"{"id": "resp_1", "output": []}"#
        )))
    );
    assert!(wire.builders().is_empty());
}

#[tokio::test]
async fn assemble_returns_none_for_opaque_sse() {
    let mut setup = fixture();
    setup.chat = Some(model_body());
    setup.anthropic_body = Some(model_body());
    let wire = FakeWire::new(setup);
    let chunks = [bytes("data: {}\n\n")];
    assert!(assemble(&wire, Surface::OpaqueSse, &chunks).await.is_none());
    assert!(wire.builders().is_empty());
}

#[test]
fn assembly_reads_the_last_terminal_event() {
    let chunks = [
        chunk(r#"{"type": "response.completed", "response": {"id": "first"}}"#),
        chunk(r#"{"type": "response.incomplete", "response": {"id": "last"}}"#),
    ];
    assert_eq!(terminal_body(&chunks), Some(value(r#"{"id": "last"}"#)));
}

#[test]
fn assembly_ignores_events_before_the_terminal_one() {
    let chunks = [chunk(
        r#"{"type": "response.created", "response": {"id": "resp_1", "output": []}}"#,
    )];
    assert_eq!(terminal_body(&chunks), None);
}

#[test]
fn assembly_ignores_a_terminal_event_without_an_object_response() {
    let chunks = [chunk(r#"{"type": "response.failed", "response": "boom"}"#)];
    assert_eq!(terminal_body(&chunks), None);
}

#[test]
fn assembly_reads_the_last_terminal_event_that_carries_a_body() {
    let chunks = [
        chunk(r#"{"type": "response.completed", "response": {"id": "kept"}}"#),
        chunk(r#"{"type": "response.failed", "response": "boom"}"#),
    ];
    assert_eq!(terminal_body(&chunks), Some(value(r#"{"id": "kept"}"#)));
}

#[test]
fn assembly_reads_a_failed_terminal_event_body() {
    let chunks = [chunk(
        r#"{"type": "response.failed", "response": {"id": "resp_1"}}"#,
    )];
    assert_eq!(terminal_body(&chunks), Some(value(r#"{"id": "resp_1"}"#)));
}

#[test]
fn assembly_ignores_chunks_that_are_not_json() {
    let chunks = [bytes("data: {\"type\": \"response.completed\"}\n\n")];
    assert_eq!(terminal_body(&chunks), None);
}
