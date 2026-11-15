use super::support::{CountingIds, content_start, ids, json, projection, session, tag};
use crate::realtime_output::events::{DeltaType, OutputEvent, OutputIds};

const USER_TEXT_SPECULATIVE: &str = r#"{"role":"USER","type":"TEXT","additionalModelFields":"{\"generationStage\":\"SPECULATIVE\"}"}"#;

pub(super) fn assistant_content_start() {
    other_roles_emit_nothing();
    user_text_marks_the_transcript_active();
    other_starts_keep_the_transcript_state();
    an_empty_response_id_is_replaced_without_created();
    an_absent_response_id_sends_created();
    existing_ids_are_kept();
}

fn assert_passes_through(text: &str) {
    let source = CountingIds::new();
    let mut state = session();
    let current = OutputIds {
        response: Some(String::new()),
        item: Some("item_old".to_owned()),
        conversation: None,
    };
    let started = projection(&mut state, &source)
        .transform_content_start_event(&content_start(text), &current);
    assert!(started.events.is_empty(), "{text}");
    assert_eq!(started.ids, current, "{text}");
    assert_eq!(started.delta_type, None, "{text}");
    assert_eq!(source.used(), 0, "{text}");
    assert!(!state.transcript.active, "{text}");
}

fn other_roles_emit_nothing() {
    assert_passes_through(r#"{"role":"SYSTEM","type":"TEXT"}"#);
    assert_passes_through(r#"{"role":"assistant","type":"TEXT"}"#);
    assert_passes_through(r#"{"type":"TEXT"}"#);
    assert_passes_through(r#"{"role":"USER","type":"AUDIO"}"#);
    assert_passes_through(r#"{"role":"USER"}"#);
    assert_passes_through(r#"{"role":"USER","type":"text"}"#);
}

fn user_text_marks_the_transcript_active() {
    let source = CountingIds::new();
    let mut state = session();
    let started = projection(&mut state, &source).transform_content_start_event(
        &content_start(USER_TEXT_SPECULATIVE),
        &OutputIds::default(),
    );
    assert!(started.events.is_empty());
    assert_eq!(started.ids, OutputIds::default());
    assert_eq!(started.delta_type, None);
    assert!(state.transcript.active);
    assert_eq!(
        state.transcript.generation_stage.as_deref(),
        Some("SPECULATIVE")
    );
    let plain = content_start(r#"{"role":"USER","type":"TEXT"}"#);
    projection(&mut state, &source).transform_content_start_event(&plain, &OutputIds::default());
    assert!(state.transcript.active);
    assert_eq!(state.transcript.generation_stage, None);
}

fn other_starts_keep_the_transcript_state() {
    let source = CountingIds::new();
    let mut state = session();
    state.transcript.active = true;
    state.transcript.generation_stage = Some("FINAL".to_owned());
    let noisy =
        r#"{"role":"SYSTEM","type":"TEXT","additionalModelFields":"{\"generationStage\":\"X\"}"}"#;
    for text in [noisy, r#"{"role":"USER","type":"AUDIO"}"#, "{}"] {
        let started = projection(&mut state, &source)
            .transform_content_start_event(&content_start(text), &OutputIds::default());
        assert!(started.events.is_empty());
    }
    assert!(state.transcript.active);
    assert_eq!(state.transcript.generation_stage.as_deref(), Some("FINAL"));
}

fn an_empty_response_id_is_replaced_without_created() {
    let source = CountingIds::new();
    let mut state = session();
    let current = OutputIds {
        response: Some(String::new()),
        item: Some("item_old".to_owned()),
        conversation: Some(String::new()),
    };
    let started = projection(&mut state, &source)
        .transform_content_start_event(&content_start(r#"{"role":"ASSISTANT"}"#), &current);
    assert_eq!(started.ids.response, Some(tag("resp", 1)));
    assert_eq!(started.ids.item.as_deref(), Some("item_old"));
    assert_eq!(started.ids.conversation, Some(tag("conv", 2)));
    assert_eq!(started.delta_type, Some(DeltaType::Text));
    assert_eq!(started.events.len(), 2);
    assert!(matches!(
        started.events.first(),
        Some(OutputEvent::OutputItemAdded(_))
    ));
    assert!(matches!(
        started.events.get(1),
        Some(OutputEvent::ContentPartAdded(_))
    ));
    assert_eq!(source.used(), 4);
}

fn an_absent_response_id_sends_created() {
    let source = CountingIds::new();
    let mut state = session();
    let current = ids(None, Some("item_old"));
    let started = projection(&mut state, &source)
        .transform_content_start_event(&content_start(r#"{"role":"ASSISTANT"}"#), &current);
    assert_eq!(started.events.len(), 3);
    let Some(OutputEvent::ResponseCreated(created)) = started.events.first() else {
        panic!("the first event is response.created");
    };
    assert_eq!(created.event_id, tag("event", 3));
    assert_eq!(created.response.id, tag("resp", 1));
    assert_eq!(created.response.conversation_id, tag("conv", 2));
}

fn existing_ids_are_kept() {
    let source = CountingIds::new();
    let mut state = session();
    let current = OutputIds {
        response: Some("resp_old".to_owned()),
        item: Some("item_old".to_owned()),
        conversation: Some("conv_old".to_owned()),
    };
    let started = projection(&mut state, &source)
        .transform_content_start_event(&content_start(r#"{"role":"ASSISTANT"}"#), &current);
    assert_eq!(started.ids, current);
    assert_eq!(started.events.len(), 2);
    assert_eq!(source.used(), 2);
}

pub(super) fn assistant_event_order() {
    let source = CountingIds::new();
    let mut state = session();
    let started = projection(&mut state, &source).transform_content_start_event(
        &content_start(r#"{"role":"ASSISTANT","type":"TEXT"}"#),
        &OutputIds::default(),
    );
    let created = format!(
        r#"{{"type":"response.created","event_id":"{}","response":{{"object":"realtime.response","id":"{}","status":"in_progress","output":[],"conversation_id":"{}"}}}}"#,
        tag("event", 4),
        tag("resp", 1),
        tag("conv", 3),
    );
    let item_added = format!(
        r#"{{"type":"response.output_item.added","response_id":"{}","output_index":0,"item":{{"id":"{}","object":"realtime.item","type":"message","status":"in_progress","role":"assistant","content":[]}}}}"#,
        tag("resp", 1),
        tag("item", 2),
    );
    let part_added = format!(
        r#"{{"type":"response.content_part.added","content_index":0,"output_index":0,"event_id":"{}","item_id":"{}","part":{{"type":"text","text":""}},"response_id":"{}"}}"#,
        tag("event", 5),
        tag("item", 2),
        tag("resp", 1),
    );
    let serialized: Vec<String> = started.events.iter().map(json).collect();
    assert_eq!(serialized, [created, item_added, part_added]);
    assert_eq!(started.delta_type, Some(DeltaType::Text));
    audio_blocks_use_an_audio_part();
    repeated_blocks_repeat_item_and_part();
}

fn audio_blocks_use_an_audio_part() {
    assert_part_kind(r#"{"role":"ASSISTANT","type":"AUDIO"}"#, DeltaType::Audio);
    assert_part_kind(r#"{"role":"ASSISTANT","type":"text"}"#, DeltaType::Audio);
    assert_part_kind(r#"{"role":"ASSISTANT"}"#, DeltaType::Text);
}

fn assert_part_kind(text: &str, expected: DeltaType) {
    let source = CountingIds::new();
    let mut state = session();
    let started = projection(&mut state, &source)
        .transform_content_start_event(&content_start(text), &ids(Some("r"), Some("i")));
    let Some(OutputEvent::ContentPartAdded(added)) = started.events.get(1) else {
        panic!("the second event is response.content_part.added");
    };
    let expected_part = match expected {
        DeltaType::Text => r#"{"type":"text","text":""}"#,
        DeltaType::Audio => r#"{"type":"audio","transcript":""}"#,
    };
    assert_eq!(json(&added.part), expected_part, "{text}");
    assert_eq!(started.delta_type, Some(expected), "{text}");
}

fn repeated_blocks_repeat_item_and_part() {
    let source = CountingIds::new();
    let mut state = session();
    let text = content_start(r#"{"role":"ASSISTANT","type":"TEXT"}"#);
    let audio = content_start(r#"{"role":"ASSISTANT","type":"AUDIO"}"#);
    let first =
        projection(&mut state, &source).transform_content_start_event(&text, &OutputIds::default());
    let second = projection(&mut state, &source).transform_content_start_event(&audio, &first.ids);
    assert_eq!(second.ids, first.ids);
    assert_eq!(second.events.len(), 2);
    assert_eq!(second.events.first(), first.events.get(1));
    assert_eq!(second.delta_type, Some(DeltaType::Audio));
    assert_eq!(source.used(), 7);
}
