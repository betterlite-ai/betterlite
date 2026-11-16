use crate::api::chat_response_stream_items::error::StreamItemsError;
use crate::api::chat_response_stream_items::items::queue_message_item_added_events;
use crate::api::chat_response_stream_items::progression::{
    is_stream_finished, return_default_done_events, return_default_initial_events,
};
use crate::api::chat_response_stream_tools::error::StreamToolsError;

use super::fixtures::{completion, event_json, field, json, sources, state_with};

fn empty_completion() -> betterlite_models::decisions::Json {
    completion(&json("\"\""), "[]")
}

#[test]
fn default_done_progression() {
    let mut state = state_with("{}");
    let sources = sources(10, &["u"]);
    let first = return_default_done_events(&mut state, &sources, &empty_completion())
        .expect("the empty completion succeeds");
    assert!(first.is_none());
    assert!(state.progress.sent_output_text_done);
    assert!(state.progress.sent_content_part_done);
    assert!(state.progress.sent_output_item_done);
    assert!(!state.progress.sent_message_item_added);
    assert!(state.progress.pending_response_events.is_empty());
    assert!(is_stream_finished(&state));
    assert!(
        return_default_done_events(&mut state, &sources, &empty_completion())
            .expect("the finished stream succeeds")
            .is_none()
    );
}

#[test]
fn default_done_progression_queues_the_message_for_final_content() {
    let completion = completion(&json("\"hi\""), "[]");
    let mut state = state_with("{}");
    let sources = sources(10, &["u"]);
    let shell = return_default_done_events(&mut state, &sources, &completion)
        .expect("the message shell builds")
        .expect("the first call returns the message shell");
    assert_eq!(
        event_json(&shell),
        json(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_u","type":"message","status":"in_progress","role":"assistant","content":[]},"sequence_number":1}"#
        )
    );
    assert_eq!(
        state.progress.pending_response_events.len(),
        1,
        "the content part added event stays queued"
    );
    assert_eq!(
        event_json(&state.progress.pending_response_events[0]),
        json(
            r#"{"type":"response.content_part.added","item_id":"msg_u","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]},"sequence_number":2}"#
        )
    );
    state.progress.litellm_model_response = Some(completion(&json("\"stored\""), "[]"));
    let text = return_default_done_events(&mut state, &sources, &completion)
        .expect("the text done event builds")
        .expect("the second call returns text done");
    assert_eq!(
        event_json(&text),
        json(
            r#"{"type":"response.output_text.done","item_id":"msg_u","output_index":0,"content_index":0,"text":"hi"}"#
        )
    );
    assert!(state.progress.sent_output_text_done);
    let part = return_default_done_events(&mut state, &sources, &completion)
        .expect("the part done event builds")
        .expect("the third call returns part done");
    assert_eq!(
        field(&event_json(&part), "type"),
        json("\"response.content_part.done\"")
    );
    assert!(state.progress.sent_content_part_done);
    let item = return_default_done_events(&mut state, &sources, &completion)
        .expect("the item done event builds")
        .expect("the fourth call returns item done");
    assert_eq!(
        event_json(&item),
        json(
            r#"{"type":"response.output_item.done","output_index":0,"sequence_number":1,"item":{"id":"msg_u","status":"completed","type":"message","role":"assistant","content":[{"type":"output_text","text":"stored","annotations":[]}]}}"#
        )
    );
    assert!(state.progress.sent_output_item_done);
    assert!(
        return_default_done_events(&mut state, &sources, &completion)
            .expect("the last call succeeds")
            .is_none()
    );
}

#[test]
fn default_done_progression_still_completes_an_added_message_with_empty_content() {
    let mut state = state_with("{}");
    let sources = sources(10, &["u"]);
    queue_message_item_added_events(&mut state, &sources).expect("the message queues");
    state.progress.litellm_model_response = Some(completion(&json("\"stored\""), "[]"));
    let text = return_default_done_events(&mut state, &sources, &empty_completion())
        .expect("the text done event builds")
        .expect("an added message still gets text done");
    assert_eq!(field(&event_json(&text), "text"), json("\"\""));
    let part = return_default_done_events(&mut state, &sources, &empty_completion())
        .expect("the part done event builds")
        .expect("an added message still gets part done");
    assert_eq!(
        field(&field(&event_json(&part), "part"), "text"),
        json("\"\"")
    );
    let item = return_default_done_events(&mut state, &sources, &empty_completion())
        .expect("the item done event builds")
        .expect("an added message still gets item done");
    assert_eq!(
        field(&field(&event_json(&item), "item"), "status"),
        json("\"completed\"")
    );
    assert!(is_stream_finished(&state));
}

#[test]
fn default_done_progression_reports_a_missing_stored_completion() {
    let mut state = state_with("{}");
    let sources = sources(10, &["u"]);
    assert!(
        return_default_done_events(&mut state, &sources, &completion(&json("\"hi\""), "[]"))
            .expect("the message shell builds")
            .is_some()
    );
    assert!(
        return_default_done_events(&mut state, &sources, &completion(&json("\"hi\""), "[]"))
            .expect("the text done event builds")
            .is_some()
    );
    assert!(
        return_default_done_events(&mut state, &sources, &completion(&json("\"hi\""), "[]"))
            .expect("the part done event builds")
            .is_some()
    );
    let result =
        return_default_done_events(&mut state, &sources, &completion(&json("\"hi\""), "[]"));
    assert!(matches!(
        result,
        Err(StreamItemsError::MissingModelResponse)
    ));
    assert!(state.progress.sent_output_item_done);
}

#[test]
fn default_done_progression_starts_with_created_then_in_progress() {
    let mut state = state_with("{}");
    let sources = sources(10, &["u"]);
    let created = return_default_initial_events(&mut state, &sources)
        .expect("the created event builds")
        .expect("the first call creates the response");
    assert_eq!(
        field(&event_json(&created), "type"),
        json("\"response.created\"")
    );
    assert!(state.progress.sent_response_created);
    let in_progress = return_default_initial_events(&mut state, &sources)
        .expect("the in-progress event builds")
        .expect("the second call starts the response");
    assert_eq!(
        field(&event_json(&in_progress), "type"),
        json("\"response.in_progress\"")
    );
    assert!(state.progress.sent_in_progress);
    assert!(
        return_default_initial_events(&mut state, &sources)
            .expect("the third call succeeds")
            .is_none()
    );
}

#[test]
fn default_done_progression_resumes_from_a_sent_created_event() {
    let mut state = state_with("{}");
    state.progress.sent_response_created = true;
    let event = return_default_initial_events(&mut state, &sources(10, &["u"]))
        .expect("the in-progress event builds")
        .expect("the in-progress event is the only one left");
    assert_eq!(
        field(&event_json(&event), "type"),
        json("\"response.in_progress\"")
    );
    assert_eq!(field(&event_json(&event), "sequence_number"), json("1"));
}

#[test]
fn default_done_progression_finishes_only_with_every_done_flag() {
    let mut state = state_with("{}");
    assert!(!is_stream_finished(&state));
    state.progress.sent_output_text_done = true;
    state.progress.sent_content_part_done = true;
    assert!(!is_stream_finished(&state));
    state.progress.finished = true;
    assert!(
        !is_stream_finished(&state),
        "the finished field is not a done flag"
    );
    state.progress.sent_output_item_done = true;
    assert!(is_stream_finished(&state));
    state.progress.finished = false;
    assert!(is_stream_finished(&state));
}

#[test]
fn default_done_progression_reports_sequence_overflow() {
    let mut state = state_with("{}");
    state.progress.sequence_number = i64::MAX;
    let result = return_default_initial_events(&mut state, &sources(10, &["u"]));
    assert!(matches!(
        result,
        Err(StreamItemsError::Stream(StreamToolsError::SequenceOverflow))
    ));
}
