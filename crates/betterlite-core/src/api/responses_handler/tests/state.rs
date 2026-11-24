use betterlite_context::non_default_params::Params;

use super::support::{
    Fixture, completed_chunk, created_chunk, delta_chunk, done_chunk, fallback_id,
    in_progress_chunk, interaction_state, object, params, unknown_chunk, value,
};
use crate::api::responses_handler::state::InteractionsStreamState;

#[test]
fn state_new_initializes_the_iterator_record() {
    let fixture = Fixture::new("gpt-4o");
    let setup = fixture.setup();
    let state = InteractionsStreamState::new(&setup);
    assert_eq!(state.model, "gpt-4o");
    assert_eq!(state.request_input, None);
    assert_eq!(state.optional_params, Params::new());
    assert_eq!(state.custom_llm_provider, None);
    assert_eq!(state.litellm_metadata, Params::new());
    assert_eq!(state.collected_text, "");
    assert!(!state.sent_interaction_start.0);
    assert!(!state.sent_content_start.0);
    assert!(!state.sent_completion_event.0);
    assert!(!state.finished.0);
    assert!(state.pending_events.is_empty());
    assert_eq!(state.interaction_id, None);
    assert_eq!(state.fallback_id, fallback_id());
}

#[test]
fn state_new_copies_the_setup_arguments() {
    let mut fixture = Fixture::new("claude-3");
    fixture.input = Some(value(r#""hello""#));
    fixture.optional_params = params(r#"{"stream": true}"#);
    fixture.custom_llm_provider = Some(String::from("anthropic"));
    fixture.metadata = params(r#"{"team": "x"}"#);
    let setup = fixture.setup();
    let state = InteractionsStreamState::new(&setup);
    assert_eq!(state.model, "claude-3");
    assert_eq!(state.request_input, Some(value(r#""hello""#)));
    assert_eq!(state.optional_params, params(r#"{"stream": true}"#));
    assert_eq!(state.custom_llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(state.litellm_metadata, params(r#"{"team": "x"}"#));
}

#[test]
fn events_for_chunk_first_delta_synthesizes_start_events() {
    let mut state = interaction_state("gpt-4o");
    let events = state
        .events_for_chunk(&delta_chunk("item_1", "hello"))
        .expect("the chunk stays under the limits");
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].event_type.as_deref(), Some("interaction.created"));
    assert_eq!(events[0].id.as_deref(), Some("item_1"));
    assert_eq!(events[0].model.as_deref(), Some("gpt-4o"));
    assert_eq!(events[1].event_type.as_deref(), Some("step.start"));
    assert_eq!(events[2].event_type.as_deref(), Some("step.delta"));
    assert_eq!(
        events[2].delta,
        Some(object(r#"{"type": "text", "text": "hello"}"#))
    );
    assert!(state.sent_interaction_start.0);
    assert!(state.sent_content_start.0);
    assert_eq!(state.interaction_id.as_deref(), Some("item_1"));
}

#[test]
fn events_for_chunk_later_delta_emits_only_the_delta() {
    let mut state = interaction_state("gpt-4o");
    assert_eq!(
        state
            .events_for_chunk(&delta_chunk("item_1", "hell"))
            .expect("the chunk stays under the limits")
            .len(),
        3
    );
    let events = state
        .events_for_chunk(&delta_chunk("item_1", "o"))
        .expect("the chunk stays under the limits");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type.as_deref(), Some("step.delta"));
    assert_eq!(
        events[0].delta,
        Some(object(r#"{"type": "text", "text": "o"}"#))
    );
}

#[test]
fn collected_text_accumulates_across_deltas() {
    let mut state = interaction_state("gpt-4o");
    assert_eq!(
        state
            .events_for_chunk(&delta_chunk("item_1", "a"))
            .expect("the chunk stays under the limits")
            .len(),
        3
    );
    assert_eq!(
        state
            .events_for_chunk(&delta_chunk("item_1", "b"))
            .expect("the chunk stays under the limits")
            .len(),
        1
    );
    assert_eq!(
        state
            .events_for_chunk(&delta_chunk("item_1", "c"))
            .expect("the chunk stays under the limits")
            .len(),
        1
    );
    assert_eq!(state.collected_text, "abc");
}

#[test]
fn events_for_chunk_delta_without_item_id_uses_the_fallback_id() {
    let mut state = interaction_state("gpt-4o");
    let events = state
        .events_for_chunk(&delta_chunk("", "hi"))
        .expect("the chunk stays under the limits");
    assert_eq!(events[0].id.as_deref(), Some(fallback_id().as_str()));
    assert_eq!(
        state.interaction_id.as_deref(),
        Some(fallback_id().as_str())
    );
}

#[test]
fn events_for_chunk_created_sends_the_interaction_start_once() {
    let mut state = interaction_state("gpt-4o");
    let events = state
        .events_for_chunk(&created_chunk("resp_1"))
        .expect("the chunk stays under the limits");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type.as_deref(), Some("interaction.created"));
    assert_eq!(events[0].id.as_deref(), Some("resp_1"));
    assert_eq!(events[0].status.as_deref(), Some("in_progress"));
    let again = state
        .events_for_chunk(&in_progress_chunk("resp_2"))
        .expect("the chunk stays under the limits");
    assert!(again.is_empty());
    assert_eq!(state.interaction_id.as_deref(), Some("resp_1"));
}

#[test]
fn events_for_chunk_in_progress_opens_the_interaction() {
    let mut state = interaction_state("gpt-4o");
    let events = state
        .events_for_chunk(&in_progress_chunk("resp_9"))
        .expect("the chunk stays under the limits");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type.as_deref(), Some("interaction.created"));
    assert_eq!(events[0].id.as_deref(), Some("resp_9"));
}

#[test]
fn events_for_chunk_ignores_chunks_it_cannot_translate() {
    let mut state = interaction_state("gpt-4o");
    assert!(
        state
            .events_for_chunk(&done_chunk())
            .expect("the chunk stays under the limits")
            .is_empty()
    );
    assert!(
        state
            .events_for_chunk(&unknown_chunk())
            .expect("the chunk stays under the limits")
            .is_empty()
    );
    assert_eq!(state.collected_text, "");
    assert!(!state.sent_interaction_start.0);
}

#[test]
fn events_for_chunk_completed_closes_content_and_completes() {
    let mut state = interaction_state("gpt-4o");
    assert_eq!(
        state
            .events_for_chunk(&delta_chunk("item_1", "hello"))
            .expect("the chunk stays under the limits")
            .len(),
        3
    );
    let terminal = state
        .events_for_chunk(&completed_chunk("resp_1"))
        .expect("the chunk stays under the limits");
    assert_eq!(terminal.len(), 2);
    assert_eq!(terminal[0].event_type.as_deref(), Some("step.stop"));
    assert_eq!(
        terminal[1].event_type.as_deref(),
        Some("interaction.completed")
    );
    assert_eq!(terminal[1].id.as_deref(), Some("item_1"));
    assert_eq!(
        terminal[1].steps,
        Some(vec![object(
            r#"{"type": "model_output", "content": [{"type": "text", "text": "hello"}]}"#
        )])
    );
    assert!(state.sent_completion_event.0);
    assert!(state.finished.0);
}

#[test]
fn events_for_chunk_completed_without_content_skips_step_stop() {
    let mut state = interaction_state("gpt-4o");
    assert_eq!(
        state
            .events_for_chunk(&created_chunk("resp_1"))
            .expect("the chunk stays under the limits")
            .len(),
        1
    );
    let terminal = state
        .events_for_chunk(&completed_chunk("resp_1"))
        .expect("the chunk stays under the limits");
    assert_eq!(terminal.len(), 1);
    assert_eq!(
        terminal[0].event_type.as_deref(),
        Some("interaction.completed")
    );
    assert_eq!(terminal[0].id.as_deref(), Some("resp_1"));
}

#[test]
fn events_for_chunk_created_without_an_id_uses_the_fallback_id() {
    let mut state = interaction_state("gpt-4o");
    let events = state
        .events_for_chunk(&created_chunk(""))
        .expect("the chunk stays under the limits");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id.as_deref(), Some(fallback_id().as_str()));
    assert_eq!(
        state.interaction_id.as_deref(),
        Some(fallback_id().as_str())
    );
}

#[test]
fn events_for_chunk_completed_without_ids_uses_the_fallback_id() {
    let mut state = interaction_state("gpt-4o");
    let terminal = state
        .events_for_chunk(&completed_chunk(""))
        .expect("the chunk stays under the limits");
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].id.as_deref(), Some(fallback_id().as_str()));
}

#[test]
fn terminal_events_on_eof_flushes_stop_and_completion() {
    let mut state = interaction_state("gpt-4o");
    assert_eq!(
        state
            .events_for_chunk(&delta_chunk("item_1", "hello"))
            .expect("the chunk stays under the limits")
            .len(),
        3
    );
    let terminal = state.terminal_events_on_eof();
    assert_eq!(terminal.len(), 2);
    assert_eq!(terminal[0].event_type.as_deref(), Some("step.stop"));
    assert_eq!(
        terminal[1].event_type.as_deref(),
        Some("interaction.completed")
    );
    assert_eq!(terminal[1].id.as_deref(), Some("item_1"));
    assert_eq!(
        terminal[1].steps,
        Some(vec![object(
            r#"{"type": "model_output", "content": [{"type": "text", "text": "hello"}]}"#
        )])
    );
    assert!(state.sent_completion_event.0);
}

#[test]
fn terminal_events_on_eof_is_empty_after_a_completion() {
    let mut state = interaction_state("gpt-4o");
    assert_eq!(
        state
            .events_for_chunk(&completed_chunk("resp_1"))
            .expect("the chunk stays under the limits")
            .len(),
        1
    );
    assert!(state.terminal_events_on_eof().is_empty());
}

#[test]
fn terminal_events_on_eof_completes_an_interaction_without_content() {
    let mut state = interaction_state("gpt-4o");
    assert_eq!(
        state
            .events_for_chunk(&created_chunk("resp_1"))
            .expect("the chunk stays under the limits")
            .len(),
        1
    );
    let terminal = state.terminal_events_on_eof();
    assert_eq!(terminal.len(), 1);
    assert_eq!(
        terminal[0].event_type.as_deref(),
        Some("interaction.completed")
    );
    assert_eq!(terminal[0].id.as_deref(), Some("resp_1"));
    assert_eq!(
        terminal[0].steps,
        Some(vec![object(
            r#"{"type": "model_output", "content": [{"type": "text", "text": ""}]}"#
        )])
    );
}

#[test]
fn terminal_events_on_eof_is_empty_before_any_event() {
    let mut state = interaction_state("gpt-4o");
    assert!(state.terminal_events_on_eof().is_empty());
    assert!(!state.sent_completion_event.0);
}
