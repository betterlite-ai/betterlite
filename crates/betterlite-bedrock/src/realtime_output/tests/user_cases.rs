use super::support::{CountingIds, content_start, ids, json, projection, session, tag};
use crate::realtime_output::events::OutputIds;
use crate::realtime_session::records::{UsageDetails, UsageEvent};

fn speech(kind: &str, event: u128, item: u128) -> String {
    format!(
        r#"{{"type":"input_audio_buffer.speech_{kind}","event_id":"{}","item_id":"{}"}}"#,
        tag("event", event),
        tag("item", item),
    )
}

fn transcript_delta(event: u128, item: u128, text: &str) -> String {
    format!(
        r#"{{"type":"conversation.item.input_audio_transcription.delta","event_id":"{}","item_id":"{}","content_index":0,"delta":"{text}"}}"#,
        tag("event", event),
        tag("item", item),
    )
}

fn transcript_completed(event: u128, item: u128, text: &str) -> String {
    format!(
        r#"{{"type":"conversation.item.input_audio_transcription.completed","event_id":"{}","item_id":"{}","content_index":0,"transcript":"{text}"}}"#,
        tag("event", event),
        tag("item", item),
    )
}

pub(super) fn user_speech_event() {
    let source = CountingIds::new();
    let mut state = session();
    state.transcript.buffer = "kept".to_owned();
    let mut output = projection(&mut state, &source);
    assert_eq!(
        json(&output.transform_user_speech_event(true)),
        speech("started", 1, 2)
    );
    assert_eq!(
        json(&output.transform_user_speech_event(true)),
        speech("started", 3, 4)
    );
    assert_eq!(
        json(&output.transform_user_speech_event(false)),
        speech("stopped", 5, 4)
    );
    assert_eq!(source.used(), 5);
    assert_eq!(state.transcript.buffer, "kept");
    assert_eq!(state.transcript.item_id, Some(tag("item", 4)));
    stop_on_a_fresh_session_allocates_an_item();
}

fn stop_on_a_fresh_session_allocates_an_item() {
    let source = CountingIds::new();
    let mut state = session();
    let mut output = projection(&mut state, &source);
    assert_eq!(
        json(&output.transform_user_speech_event(false)),
        speech("stopped", 1, 2)
    );
    assert_eq!(
        json(&output.transform_user_speech_event(false)),
        speech("stopped", 3, 2)
    );
}

pub(super) fn user_transcript_final() {
    let source = CountingIds::new();
    let mut state = session();
    let mut output = projection(&mut state, &source);
    assert_eq!(output.user_transcript_completed_events(), None);
    assert_eq!(source.used(), 0);
    let first = output.transform_user_transcript_event("hello ");
    assert_eq!(json(&first), transcript_delta(1, 2, "hello "));
    let second = output.transform_user_transcript_event("world");
    assert_eq!(json(&second), transcript_delta(3, 2, "world"));
    let completed = output.user_transcript_completed_events();
    assert_eq!(
        completed.as_ref().map(json),
        Some(transcript_completed(4, 2, "hello world"))
    );
    assert_eq!(output.user_transcript_completed_events(), None);
    assert_eq!(source.used(), 4);
    assert_eq!(state.transcript.buffer, "");
    assert_eq!(state.transcript.item_id, Some(tag("item", 2)));
    lowercase_speculative_still_appends();
}

fn lowercase_speculative_still_appends() {
    let source = CountingIds::new();
    let mut state = session();
    state.transcript.generation_stage = Some("speculative".to_owned());
    projection(&mut state, &source).transform_user_transcript_event("kept");
    assert_eq!(state.transcript.buffer, "kept");
}

pub(super) fn speculative_text() {
    let source = CountingIds::new();
    let mut state = session();
    let speculative = content_start(
        r#"{"role":"USER","type":"TEXT","additionalModelFields":"{\"generationStage\":\"SPECULATIVE\"}"}"#,
    );
    let mut output = projection(&mut state, &source);
    output.transform_content_start_event(&speculative, &OutputIds::default());
    let draft = output.transform_user_transcript_event("draft");
    assert_eq!(json(&draft), transcript_delta(1, 2, "draft"));
    assert_eq!(output.user_transcript_completed_events(), None);
    let settled = content_start(
        r#"{"role":"USER","type":"TEXT","additionalModelFields":"{\"generationStage\":\"FINAL\"}"}"#,
    );
    output.transform_content_start_event(&settled, &OutputIds::default());
    output.transform_user_transcript_event("final");
    let completed = output.user_transcript_completed_events();
    assert_eq!(
        completed.as_ref().map(json),
        Some(transcript_completed(4, 2, "final"))
    );
    assistant_text_ignores_the_generation_stage();
}

fn assistant_text_ignores_the_generation_stage() {
    let source = CountingIds::new();
    let mut state = session();
    state.transcript.generation_stage = Some("SPECULATIVE".to_owned());
    let current = ids(Some("resp_a"), Some("item_a"));
    let mut chunks = None;
    let event = projection(&mut state, &source).transform_text_output_event(
        &serde_json::from_str(r#"{"content":"visible"}"#).expect("a textOutput fixture"),
        &current,
        &mut chunks,
    );
    assert!(event.is_some());
    assert_eq!(chunks.as_ref().map(Vec::len), Some(1));
}

pub(super) fn item_id_consistency() {
    let source = CountingIds::new();
    let mut state = session();
    let mut output = projection(&mut state, &source);
    assert_eq!(
        json(&output.transform_user_speech_event(true)),
        speech("started", 1, 2)
    );
    assert_eq!(
        json(&output.transform_user_transcript_event("a")),
        transcript_delta(3, 2, "a")
    );
    assert_eq!(
        json(&output.transform_user_speech_event(false)),
        speech("stopped", 4, 2)
    );
    let completed = output.user_transcript_completed_events();
    assert_eq!(
        completed.as_ref().map(json),
        Some(transcript_completed(5, 2, "a"))
    );
    assert_eq!(
        json(&output.transform_user_speech_event(true)),
        speech("started", 6, 7)
    );
    assert_eq!(
        json(&output.transform_user_transcript_event("b")),
        transcript_delta(8, 7, "b")
    );
    assistant_ids_stay_separate();
    usage_replaces_the_snapshot();
}

fn assistant_ids_stay_separate() {
    let source = CountingIds::new();
    let mut state = session();
    state.transcript.item_id = Some("item_user".to_owned());
    let mut output = projection(&mut state, &source);
    let started = output.transform_content_start_event(
        &content_start(r#"{"role":"ASSISTANT"}"#),
        &OutputIds::default(),
    );
    assert_eq!(started.ids.item, Some(tag("item", 2)));
    output.transform_user_transcript_event("x");
    assert_eq!(state.transcript.item_id.as_deref(), Some("item_user"));
}

fn usage_replaces_the_snapshot() {
    let source = CountingIds::new();
    let mut state = session();
    let first = UsageEvent {
        total_input_tokens: 5,
        total_output_tokens: 3,
        total_tokens: 8,
        details: UsageDetails::default(),
    };
    let second = UsageEvent {
        total_input_tokens: 9,
        total_output_tokens: 0,
        total_tokens: 0,
        details: UsageDetails::default(),
    };
    let mut output = projection(&mut state, &source);
    output.transform_usage_event(first);
    output.transform_usage_event(second.clone());
    assert_eq!(state.cumulative_usage, second);
    assert_eq!(state.reported_usage, UsageEvent::default());
    assert_eq!(source.used(), 0);
}
