use super::super::events::{
    completion_event, content_start_event, content_stop_event, interaction_start_event,
    text_delta_event,
};
use super::support::expected_chunk;

#[test]
fn interaction_start_event_carries_id_object_status_and_model() {
    let event = interaction_start_event("interaction_1", "gpt-4o");
    assert_eq!(
        event,
        expected_chunk(
            r#"{
                "event_type": "interaction.created",
                "id": "interaction_1",
                "object": "interaction",
                "model": "gpt-4o",
                "status": "in_progress"
            }"#
        )
    );
}

#[test]
fn content_start_event_opens_an_empty_model_output_step() {
    let event = content_start_event();
    assert_eq!(
        event,
        expected_chunk(
            r#"{
                "event_type": "step.start",
                "object": "interaction",
                "index": 0,
                "step": {"type": "model_output", "content": []}
            }"#
        )
    );
}

#[test]
fn text_delta_event_carries_the_text_at_index_zero() {
    let event = text_delta_event("hello");
    assert_eq!(
        event,
        expected_chunk(
            r#"{
                "event_type": "step.delta",
                "object": "interaction",
                "index": 0,
                "delta": {"type": "text", "text": "hello"}
            }"#
        )
    );
}

#[test]
fn content_stop_event_sets_only_the_event_type_and_index() {
    let event = content_stop_event();
    assert_eq!(
        event,
        expected_chunk(
            r#"{
                "event_type": "step.stop",
                "object": "interaction",
                "index": 0
            }"#
        )
    );
}

#[test]
fn completion_event_carries_the_given_id_model_and_full_text() {
    let event = completion_event("interaction_1", "gpt-4o", "hello world");
    assert_eq!(
        event,
        expected_chunk(
            r#"{
                "event_type": "interaction.completed",
                "id": "interaction_1",
                "object": "interaction",
                "model": "gpt-4o",
                "status": "completed",
                "steps": [
                    {"type": "model_output", "content": [{"type": "text", "text": "hello world"}]}
                ]
            }"#
        )
    );
}
