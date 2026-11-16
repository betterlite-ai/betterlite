#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::disallowed_types,
        clippy::too_many_lines,
        reason = "the fixtures state the exact event JSON the transforms emit, field for field, and each test reads as one scenario"
    )
)]

use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_prompts::arguments::json::Json;
use serde_json::json;

use super::delta::content_delta_event;
use super::done::{additional_done_events, content_done_event};
use super::error::ContentError;
use super::events::ContentEvent;
use super::ids::{ContentIds, UuidIds};
use super::map::{
    MappedEvent, ModalityEvent, get_nested_value, map_generation_complete_event,
    map_model_turn_event, map_openai_event,
};
use super::modality::handle_openai_modality_event;
use super::start::{StartContent, start_content_events};
use super::state::ContentState;
use super::update::{ChunkKind, ChunkSource, update_current_chunks};
use uuid::Uuid;

const SETUP: &str = r#"{"setup":{"generationConfig":{"responseModalities":["AUDIO","TEXT"],"temperature":0.5,"maxOutputTokens":256}}}"#;

struct FixedIds {
    next: u64,
}

impl ContentIds for FixedIds {
    fn next_id(&mut self) -> String {
        let id = uuid(self.next);
        self.next = self.next.saturating_add(1);
        id
    }
}

const fn ids() -> FixedIds {
    FixedIds { next: 0 }
}

fn uuid(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

fn value(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn chunks(text: &str) -> Vec<Json> {
    serde_json::from_str(text).unwrap()
}

fn event(value: &ContentEvent) -> serde_json::Value {
    serde_json::to_value(value).unwrap()
}

#[test]
fn classifies_the_first_model_turn_part() {
    let text = value(r#"{"parts": [{"text": "hi"}]}"#);
    assert_eq!(map_model_turn_event(&text).unwrap(), MappedEvent::TextDelta);
    let audio = value(r#"{"parts": [{"inlineData": {"mimeType": "audio/pcm"}}]}"#);
    assert_eq!(
        map_model_turn_event(&audio).unwrap(),
        MappedEvent::AudioDelta
    );
    let inline_first = value(r#"{"parts": [{"inlineData": {"data": "QQ=="}}, {"text": "later"}]}"#);
    assert_eq!(
        map_model_turn_event(&inline_first).unwrap(),
        MappedEvent::AudioDelta
    );
}

#[test]
fn fails_on_missing_empty_and_unknown_parts() {
    let missing = value(r#"{"nothing": 1}"#);
    assert!(matches!(
        map_model_turn_event(&missing),
        Err(ContentError::MissingParts)
    ));
    let not_a_list = value(r#"{"parts": {}}"#);
    assert!(matches!(
        map_model_turn_event(&not_a_list),
        Err(ContentError::PartsNotList)
    ));
    let empty = value(r#"{"parts": []}"#);
    assert!(matches!(
        map_model_turn_event(&empty),
        Err(ContentError::EmptyParts)
    ));
    let unexpected = value(r#"{"parts": [{"thought": true}]}"#);
    assert!(matches!(
        map_model_turn_event(&unexpected),
        Err(ContentError::UnexpectedPart)
    ));
}

#[test]
fn rejects_turns_and_parts_that_are_not_objects() {
    let scalar = value("5");
    assert!(matches!(
        map_model_turn_event(&scalar),
        Err(ContentError::ModelTurnNotObject)
    ));
    let numeric_part = value(r#"{"parts": [5]}"#);
    assert!(matches!(
        map_model_turn_event(&numeric_part),
        Err(ContentError::PartNotObject)
    ));
}

#[test]
fn maps_generation_complete_only_with_a_prior_delta_type() {
    assert_eq!(
        map_generation_complete_event(Some(AllDeltaType::Text)).unwrap(),
        MappedEvent::TextDone
    );
    assert_eq!(
        map_generation_complete_event(Some(AllDeltaType::Audio)).unwrap(),
        MappedEvent::AudioDone
    );
    assert!(matches!(
        map_generation_complete_event(None),
        Err(ContentError::MissingDeltaType)
    ));
}

#[test]
fn maps_mapped_events_to_modality_events() {
    assert_eq!(
        ModalityEvent::from_mapped(MappedEvent::TextDelta),
        Some(ModalityEvent::TextDelta)
    );
    assert_eq!(ModalityEvent::TextDelta.delta_type(), AllDeltaType::Text);
    assert_eq!(
        ModalityEvent::from_mapped(MappedEvent::AudioDelta),
        Some(ModalityEvent::AudioDelta)
    );
    assert_eq!(ModalityEvent::AudioDelta.delta_type(), AllDeltaType::Audio);
    assert_eq!(
        ModalityEvent::from_mapped(MappedEvent::TextDone),
        Some(ModalityEvent::TextDone)
    );
    assert_eq!(ModalityEvent::TextDone.delta_type(), AllDeltaType::Text);
    assert_eq!(
        ModalityEvent::from_mapped(MappedEvent::AudioDone),
        Some(ModalityEvent::AudioDone)
    );
    assert_eq!(ModalityEvent::AudioDone.delta_type(), AllDeltaType::Audio);
    assert_eq!(
        ModalityEvent::from_mapped(MappedEvent::SessionCreated),
        None
    );
    assert_eq!(
        ModalityEvent::from_mapped(MappedEvent::FunctionCallArgumentsDone),
        None
    );
    assert_eq!(ModalityEvent::from_mapped(MappedEvent::ResponseDone), None);
}

#[test]
fn generates_uuid_v4_ids() {
    let mut ids = UuidIds;
    let identifier = ids.next_id();
    assert_eq!(Uuid::parse_str(&identifier).unwrap().get_version_num(), 4);
}

#[test]
fn maps_only_the_current_key_and_value() {
    let false_completion = value(r#"{"generationComplete": false, "turnComplete": true}"#);
    assert_eq!(
        map_openai_event("serverContent", &false_completion, Some(AllDeltaType::Text)).unwrap(),
        MappedEvent::TextDone
    );
    let null_completion = value(r#"{"generationComplete": null, "turnComplete": true}"#);
    assert_eq!(
        map_openai_event("serverContent", &null_completion, Some(AllDeltaType::Text)).unwrap(),
        MappedEvent::ResponseDone
    );
    let false_turn = value(r#"{"turnComplete": false}"#);
    assert_eq!(
        map_openai_event("serverContent", &false_turn, Some(AllDeltaType::Text)).unwrap(),
        MappedEvent::ResponseDone
    );
    let interrupted = value(r#"{"turnComplete": null, "interrupted": true}"#);
    assert_eq!(
        map_openai_event("serverContent", &interrupted, Some(AllDeltaType::Text)).unwrap(),
        MappedEvent::ResponseDone
    );
    let model_turn_wins =
        value(r#"{"modelTurn": {"parts": [{"text": "x"}]}, "generationComplete": true}"#);
    assert_eq!(
        map_openai_event("serverContent", &model_turn_wins, Some(AllDeltaType::Audio)).unwrap(),
        MappedEvent::TextDelta
    );
    let completion = value(r#"{"generationComplete": true}"#);
    assert_eq!(
        map_openai_event("serverContent", &completion, Some(AllDeltaType::Audio)).unwrap(),
        MappedEvent::AudioDone
    );
    let tool_call = value(r#"{"functionCalls": []}"#);
    assert_eq!(
        map_openai_event("toolCall", &tool_call, Some(AllDeltaType::Text)).unwrap(),
        MappedEvent::FunctionCallArgumentsDone
    );
    let setup = value("{}");
    assert_eq!(
        map_openai_event("setupComplete", &setup, None).unwrap(),
        MappedEvent::SessionCreated
    );
    let scalar = value("5");
    assert_eq!(
        map_openai_event("setupComplete", &scalar, None).unwrap(),
        MappedEvent::SessionCreated
    );
    assert!(matches!(
        map_openai_event("serverContent", &scalar, Some(AllDeltaType::Text)),
        Err(ContentError::UnknownEvent(key)) if key == "serverContent"
    ));
    let usage = value(r#"{"totalTokenCount": 3}"#);
    assert!(matches!(
        map_openai_event("usageMetadata", &usage, Some(AllDeltaType::Text)),
        Err(ContentError::UnknownEvent(key)) if key == "usageMetadata"
    ));
}

#[test]
fn reads_nested_values_by_dotted_path() {
    let nested = value(r#"{"a": {"b": 1}, "n": null, "t": "text"}"#);
    assert!(get_nested_value(&nested, "a.b").is_some());
    assert!(get_nested_value(&nested, "a.c").is_none());
    assert!(matches!(get_nested_value(&nested, "n"), Some(Json::Null)));
    assert!(get_nested_value(&nested, "a.b.c").is_none());
    assert_eq!(
        get_nested_value(&nested, "t").and_then(Json::as_str),
        Some("text")
    );
}

#[test]
fn starts_content_in_source_order() {
    let content = StartContent {
        setup_request: Some(SETUP),
        response_id: "resp_1",
        output_item_id: "item_1",
        conversation_id: "conv_1",
        delta_type: AllDeltaType::Text,
    };
    let events = start_content_events(&content, &mut ids()).unwrap();
    assert_eq!(
        serde_json::to_value(&events).unwrap(),
        json!([
            {
                "type": "response.created",
                "event_id": format!("event_{}", uuid(0)),
                "response": {
                    "object": "realtime.response",
                    "id": "resp_1",
                    "status": "in_progress",
                    "status_details": null,
                    "output": [],
                    "conversation_id": "conv_1",
                    "modalities": ["audio", "text"],
                    "temperature": 0.5,
                    "max_output_tokens": 256,
                },
            },
            {
                "type": "response.output_item.added",
                "event_id": format!("event_{}", uuid(1)),
                "response_id": "resp_1",
                "output_index": 0,
                "item": {
                    "id": "item_1",
                    "object": "realtime.item",
                    "type": "message",
                    "status": "in_progress",
                    "role": "assistant",
                    "content": [],
                },
            },
            {
                "type": "conversation.item.added",
                "event_id": format!("event_{}", uuid(2)),
                "previous_item_id": null,
                "item": {
                    "id": "item_1",
                    "object": "realtime.item",
                    "type": "message",
                    "status": "in_progress",
                    "role": "assistant",
                    "content": [],
                },
            },
            {
                "type": "response.content_part.added",
                "content_index": 0,
                "output_index": 0,
                "event_id": format!("event_{}", uuid(3)),
                "item_id": "item_1",
                "part": {"type": "text", "text": ""},
                "response_id": "resp_1",
            },
        ])
    );
}

#[test]
fn starts_with_default_modalities_when_setup_is_absent_or_invalid() {
    let absent = StartContent {
        setup_request: None,
        response_id: "resp_1",
        output_item_id: "item_1",
        conversation_id: "conv_1",
        delta_type: AllDeltaType::Audio,
    };
    let absent_events =
        serde_json::to_value(start_content_events(&absent, &mut ids()).unwrap()).unwrap();
    assert_eq!(absent_events[0]["response"]["modalities"], json!(["audio"]));
    assert_eq!(
        absent_events[0]["response"]["temperature"],
        serde_json::Value::Null
    );
    assert_eq!(
        absent_events[0]["response"]["max_output_tokens"],
        serde_json::Value::Null
    );
    assert_eq!(
        absent_events[3]["part"],
        json!({"type": "audio", "transcript": ""})
    );
    let invalid = StartContent {
        setup_request: Some("not json"),
        ..absent
    };
    let invalid_events =
        serde_json::to_value(start_content_events(&invalid, &mut ids()).unwrap()).unwrap();
    assert_eq!(
        invalid_events[0]["response"]["modalities"],
        json!(["audio"])
    );
    let empty_modalities = StartContent {
        setup_request: Some(r#"{"setup":{"generationConfig":{"responseModalities":[]}}}"#),
        ..invalid
    };
    let empty_events =
        serde_json::to_value(start_content_events(&empty_modalities, &mut ids()).unwrap()).unwrap();
    assert_eq!(empty_events[0]["response"]["modalities"], json!([]));
}

#[test]
fn rejects_a_setup_that_is_not_an_object() {
    let content = StartContent {
        setup_request: Some(r#"{"setup": 5}"#),
        response_id: "resp_1",
        output_item_id: "item_1",
        conversation_id: "conv_1",
        delta_type: AllDeltaType::Text,
    };
    assert!(matches!(
        start_content_events(&content, &mut ids()),
        Err(ContentError::Setup(_))
    ));
}

#[test]
fn concatenates_every_model_turn_part() {
    let message = value(
        r#"{"modelTurn": {"parts": [
            {"text": "He"},
            {"inlineData": {"data": "bG8="}},
            {"thought": true},
            {"inlineData": {"mimeType": "audio/pcm"}},
            {"text": "!"}
        ]}}"#,
    );
    let mut ids = ids();
    let text =
        content_delta_event(&message, "item_1", "resp_1", AllDeltaType::Text, &mut ids).unwrap();
    assert_eq!(
        event(&text),
        json!({
            "type": "response.output_text.delta",
            "content_index": 0,
            "event_id": format!("event_{}", uuid(0)),
            "item_id": "item_1",
            "output_index": 0,
            "response_id": "resp_1",
            "delta": "HebG8=!",
        })
    );
    let audio =
        content_delta_event(&message, "item_1", "resp_1", AllDeltaType::Audio, &mut ids).unwrap();
    assert_eq!(event(&audio)["type"], "response.output_audio.delta");
    assert_eq!(event(&audio)["delta"], "HebG8=!");
    let empty = value("{}");
    let none =
        content_delta_event(&empty, "item_1", "resp_1", AllDeltaType::Text, &mut ids).unwrap();
    assert_eq!(event(&none)["delta"], "");
}

#[test]
fn rejects_malformed_delta_frames() {
    let mut ids = ids();
    let scalar = value("5");
    assert!(matches!(
        content_delta_event(&scalar, "item_1", "resp_1", AllDeltaType::Text, &mut ids),
        Err(ContentError::ContentNotObject)
    ));
    let scalar_turn = value(r#"{"modelTurn": 5}"#);
    assert!(matches!(
        content_delta_event(
            &scalar_turn,
            "item_1",
            "resp_1",
            AllDeltaType::Text,
            &mut ids
        ),
        Err(ContentError::ModelTurnNotObject)
    ));
    let numeric_parts = value(r#"{"modelTurn": {"parts": 5}}"#);
    assert!(matches!(
        content_delta_event(
            &numeric_parts,
            "item_1",
            "resp_1",
            AllDeltaType::Text,
            &mut ids
        ),
        Err(ContentError::PartsNotList)
    ));
    let numeric_part = value(r#"{"modelTurn": {"parts": [5]}}"#);
    assert!(matches!(
        content_delta_event(
            &numeric_part,
            "item_1",
            "resp_1",
            AllDeltaType::Text,
            &mut ids
        ),
        Err(ContentError::PartNotObject)
    ));
    let numeric_text = value(r#"{"modelTurn": {"parts": [{"text": 5}]}}"#);
    assert!(matches!(
        content_delta_event(
            &numeric_text,
            "item_1",
            "resp_1",
            AllDeltaType::Text,
            &mut ids
        ),
        Err(ContentError::PartValueNotString)
    ));
    let scalar_inline = value(r#"{"modelTurn": {"parts": [{"inlineData": 5}]}}"#);
    assert!(matches!(
        content_delta_event(
            &scalar_inline,
            "item_1",
            "resp_1",
            AllDeltaType::Text,
            &mut ids
        ),
        Err(ContentError::PartNotObject)
    ));
    let numeric_data = value(r#"{"modelTurn": {"parts": [{"inlineData": {"data": 5}}]}}"#);
    assert!(matches!(
        content_delta_event(
            &numeric_data,
            "item_1",
            "resp_1",
            AllDeltaType::Text,
            &mut ids
        ),
        Err(ContentError::PartValueNotString)
    ));
}

#[test]
fn done_joins_supplied_text_chunks() {
    let chunks = chunks(
        r#"[{"type": "response.output_text.delta", "delta": "He"},
            {"type": "response.output_text.delta", "delta": "llo"}]"#,
    );
    let mut ids = ids();
    let done = content_done_event(
        Some(chunks.as_slice()),
        Some("item_1"),
        Some("resp_1"),
        AllDeltaType::Text,
        &mut ids,
    )
    .unwrap();
    assert_eq!(
        event(&done.event),
        json!({
            "type": "response.output_text.done",
            "content_index": 0,
            "event_id": format!("event_{}", uuid(0)),
            "item_id": "item_1",
            "output_index": 0,
            "response_id": "resp_1",
            "text": "Hello",
        })
    );
    let additional = additional_done_events(&done, &mut ids);
    assert_eq!(
        serde_json::to_value(&additional).unwrap(),
        json!([
            {
                "type": "response.content_part.done",
                "content_index": 0,
                "event_id": format!("event_{}", uuid(1)),
                "item_id": "item_1",
                "output_index": 0,
                "part": {"type": "text", "text": "Hello"},
                "response_id": "resp_1",
            },
            {
                "type": "response.output_item.done",
                "event_id": format!("event_{}", uuid(2)),
                "item": {
                    "id": "item_1",
                    "object": "realtime.item",
                    "type": "message",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "Hello"}],
                },
                "output_index": 0,
                "response_id": "resp_1",
            },
        ])
    );
}

#[test]
fn empty_completed_text_yields_an_audio_part() {
    let empty_text = chunks(r#"[{"delta": ""}, {"delta": ""}]"#);
    let mut ids = ids();
    let done = content_done_event(
        Some(empty_text.as_slice()),
        Some("item_1"),
        Some("resp_1"),
        AllDeltaType::Text,
        &mut ids,
    )
    .unwrap();
    assert_eq!(event(&done.event)["text"], "");
    let text_additional = additional_done_events(&done, &mut ids);
    assert_eq!(
        serde_json::to_value(&text_additional).unwrap()[0]["part"],
        json!({"type": "audio", "transcript": ""})
    );
    assert_eq!(
        serde_json::to_value(&text_additional).unwrap()[1]["item"]["content"],
        json!([{"type": "audio", "transcript": ""}])
    );
    let audio_done = content_done_event(
        None,
        Some("item_1"),
        Some("resp_1"),
        AllDeltaType::Audio,
        &mut ids,
    )
    .unwrap();
    let audio_done_value = event(&audio_done.event);
    assert_eq!(audio_done_value["type"], "response.output_audio.done");
    assert!(audio_done_value.get("text").is_none());
    let audio_additional = additional_done_events(&audio_done, &mut ids);
    assert_eq!(
        serde_json::to_value(&audio_additional).unwrap()[0]["part"],
        json!({"type": "audio", "transcript": ""})
    );
}

#[test]
fn rejects_malformed_done_chunks() {
    let mut ids = ids();
    let scalar = chunks("[5]");
    assert!(matches!(
        content_done_event(
            Some(scalar.as_slice()),
            Some("item_1"),
            Some("resp_1"),
            AllDeltaType::Text,
            &mut ids
        ),
        Err(ContentError::DeltaChunkNotObject)
    ));
    let missing = chunks(r#"[{"type": "response.output_text.delta"}]"#);
    assert!(matches!(
        content_done_event(
            Some(missing.as_slice()),
            Some("item_1"),
            Some("resp_1"),
            AllDeltaType::Text,
            &mut ids
        ),
        Err(ContentError::MissingDeltaField)
    ));
    let numeric = chunks(r#"[{"delta": 5}]"#);
    assert!(matches!(
        content_done_event(
            Some(numeric.as_slice()),
            Some("item_1"),
            Some("resp_1"),
            AllDeltaType::Text,
            &mut ids
        ),
        Err(ContentError::MissingDeltaField)
    ));
}

#[test]
fn done_generates_missing_ids_and_additional_events_reuse_them() {
    let mut ids = ids();
    let done = content_done_event(None, None, None, AllDeltaType::Text, &mut ids).unwrap();
    let done_value = event(&done.event);
    assert_eq!(done_value["item_id"], format!("item_{}", uuid(0)));
    assert_eq!(done_value["response_id"], format!("resp_{}", uuid(1)));
    assert_eq!(done_value["event_id"], format!("event_{}", uuid(2)));
    assert_eq!(done.item_id, format!("item_{}", uuid(0)));
    assert_eq!(done.response_id, format!("resp_{}", uuid(1)));
    let additional = additional_done_events(&done, &mut ids);
    let additional_value = serde_json::to_value(&additional).unwrap();
    assert_eq!(additional_value[0]["item_id"], format!("item_{}", uuid(0)));
    assert_eq!(
        additional_value[0]["response_id"],
        format!("resp_{}", uuid(1))
    );
    assert_eq!(
        additional_value[0]["event_id"],
        format!("event_{}", uuid(3))
    );
    assert_eq!(
        additional_value[1]["item"]["id"],
        format!("item_{}", uuid(0))
    );
    assert_eq!(
        additional_value[1]["event_id"],
        format!("event_{}", uuid(4))
    );
}

#[test]
fn delta_events_start_content_and_record_state() {
    let frame = value(r#"{"serverContent": {"modelTurn": {"parts": [{"text": "hi"}]}}}"#);
    let mut state = ContentState::default();
    let mut ids = ids();
    let events =
        handle_openai_modality_event(&mut state, ModalityEvent::TextDelta, &frame, None, &mut ids)
            .unwrap();
    assert_eq!(events.len(), 5);
    let mapped = serde_json::to_value(&events).unwrap();
    assert_eq!(mapped[0]["type"], "response.created");
    assert_eq!(mapped[1]["type"], "response.output_item.added");
    assert_eq!(mapped[2]["type"], "conversation.item.added");
    assert_eq!(mapped[3]["type"], "response.content_part.added");
    assert_eq!(mapped[4]["type"], "response.output_text.delta");
    assert_eq!(mapped[4]["delta"], "hi");
    assert_eq!(mapped[4]["event_id"], format!("event_{}", uuid(7)));
    assert_eq!(state.response_id, Some(format!("resp_{}", uuid(0))));
    assert_eq!(state.output_item_id, Some(format!("item_{}", uuid(1))));
    assert_eq!(state.conversation_id, Some(format!("conv_{}", uuid(2))));
    assert_eq!(state.delta_type, Some(AllDeltaType::Text));
}

#[test]
fn delta_events_reuse_present_ids_without_starting_content() {
    let frame =
        value(r#"{"serverContent": {"modelTurn": {"parts": [{"inlineData": {"data": "QQ=="}}]}}}"#);
    let mut state = ContentState {
        output_item_id: Some("item_keep".to_owned()),
        response_id: None,
        conversation_id: None,
        delta_chunks: None,
        item_chunks: None,
        delta_type: None,
    };
    let mut first_ids = ids();
    let events = handle_openai_modality_event(
        &mut state,
        ModalityEvent::AudioDelta,
        &frame,
        None,
        &mut first_ids,
    )
    .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(event(&events[0])["type"], "response.output_audio.delta");
    assert_eq!(state.response_id, Some(format!("resp_{}", uuid(0))));
    assert_eq!(state.output_item_id.as_deref(), Some("item_keep"));
    assert_eq!(state.conversation_id, None);
    let mut kept_state = ContentState {
        output_item_id: Some("item_keep".to_owned()),
        response_id: Some("resp_keep".to_owned()),
        conversation_id: Some("conv_keep".to_owned()),
        delta_chunks: None,
        item_chunks: None,
        delta_type: None,
    };
    let kept_events = handle_openai_modality_event(
        &mut kept_state,
        ModalityEvent::TextDelta,
        &frame,
        None,
        &mut ids(),
    )
    .unwrap();
    assert_eq!(kept_events.len(), 1);
    assert_eq!(kept_state.response_id.as_deref(), Some("resp_keep"));
    assert_eq!(kept_state.conversation_id.as_deref(), Some("conv_keep"));
}

#[test]
fn delta_events_reuse_the_conversation_id_when_starting_new_content() {
    let frame = value(r#"{"serverContent": {"modelTurn": {"parts": [{"text": "hi"}]}}}"#);
    let mut state = ContentState {
        output_item_id: None,
        response_id: Some("resp_keep".to_owned()),
        conversation_id: Some("conv_keep".to_owned()),
        delta_chunks: None,
        item_chunks: None,
        delta_type: None,
    };
    let mut ids = ids();
    let events =
        handle_openai_modality_event(&mut state, ModalityEvent::TextDelta, &frame, None, &mut ids)
            .unwrap();
    assert_eq!(events.len(), 5);
    let mapped = serde_json::to_value(&events).unwrap();
    assert_eq!(mapped[0]["response"]["id"], "resp_keep");
    assert_eq!(mapped[0]["response"]["conversation_id"], "conv_keep");
    assert_eq!(mapped[0]["event_id"], format!("event_{}", uuid(1)));
    assert_eq!(mapped[1]["item"]["id"], format!("item_{}", uuid(0)));
    assert_eq!(state.output_item_id, Some(format!("item_{}", uuid(0))));
    assert_eq!(state.response_id.as_deref(), Some("resp_keep"));
    assert_eq!(state.conversation_id.as_deref(), Some("conv_keep"));
}

#[test]
fn delta_events_replace_empty_ids() {
    let frame =
        value(r#"{"serverContent": {"modelTurn": {"parts": [{"inlineData": {"data": "QQ=="}}]}}}"#);
    let mut state = ContentState {
        output_item_id: Some(String::new()),
        response_id: Some(String::new()),
        conversation_id: Some(String::new()),
        delta_chunks: None,
        item_chunks: None,
        delta_type: None,
    };
    let mut ids = ids();
    let events = handle_openai_modality_event(
        &mut state,
        ModalityEvent::AudioDelta,
        &frame,
        None,
        &mut ids,
    )
    .unwrap();
    assert_eq!(events.len(), 5);
    let mapped = serde_json::to_value(&events).unwrap();
    assert_eq!(mapped[0]["response"]["id"], format!("resp_{}", uuid(0)));
    assert_eq!(
        mapped[0]["response"]["conversation_id"],
        format!("conv_{}", uuid(2))
    );
    assert_eq!(mapped[1]["item"]["id"], format!("item_{}", uuid(1)));
    assert_eq!(state.output_item_id, Some(format!("item_{}", uuid(1))));
    assert_eq!(state.response_id, Some(format!("resp_{}", uuid(0))));
    assert_eq!(state.conversation_id, Some(format!("conv_{}", uuid(2))));
}

#[test]
fn delta_events_require_server_content() {
    let mut state = ContentState::default();
    let without_server_content = value(r#"{"setupComplete": {}}"#);
    let mut ids = ids();
    assert!(matches!(
        handle_openai_modality_event(
            &mut state,
            ModalityEvent::TextDelta,
            &without_server_content,
            None,
            &mut ids
        ),
        Err(ContentError::MissingServerContent)
    ));
    assert!(matches!(
        handle_openai_modality_event(
            &mut state,
            ModalityEvent::AudioDelta,
            &Json::Null,
            None,
            &mut ids
        ),
        Err(ContentError::MissingServerContent)
    ));
}

#[test]
fn done_events_do_not_replace_caller_state_ids() {
    let frame = value(r#"{"serverContent": {"generationComplete": true}}"#);
    let mut state = ContentState {
        output_item_id: None,
        response_id: None,
        conversation_id: None,
        delta_chunks: Some(chunks(r#"[{"delta": "Hi"}]"#)),
        item_chunks: None,
        delta_type: Some(AllDeltaType::Text),
    };
    let mut ids = ids();
    let events =
        handle_openai_modality_event(&mut state, ModalityEvent::TextDone, &frame, None, &mut ids)
            .unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(event(&events[0])["type"], "response.output_text.done");
    assert_eq!(event(&events[0])["text"], "Hi");
    assert_eq!(event(&events[0])["item_id"], format!("item_{}", uuid(0)));
    assert_eq!(
        event(&events[0])["response_id"],
        format!("resp_{}", uuid(1))
    );
    assert_eq!(event(&events[1])["item_id"], format!("item_{}", uuid(0)));
    assert_eq!(state.output_item_id, None);
    assert_eq!(state.response_id, None);
    assert_eq!(state.conversation_id, None);
    assert_eq!(state.delta_type, Some(AllDeltaType::Text));
}

#[test]
fn list_chunk_updates_replace_and_single_text_deltas_append() {
    let text_a = text_chunk("a");
    let first = update_current_chunks(ChunkSource::Single(&text_a), None, ChunkKind::Delta);
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        json!([{"type": "response.output_text.delta", "delta": "a"}])
    );
    let text_b = text_chunk("b");
    let second = update_current_chunks(ChunkSource::Single(&text_b), first, ChunkKind::Delta);
    assert_eq!(
        serde_json::to_value(&second).unwrap(),
        json!([
            {"type": "response.output_text.delta", "delta": "a"},
            {"type": "response.output_text.delta", "delta": "b"},
        ])
    );
    let list = [text_chunk("c"), audio_chunk(), text_chunk("d")];
    let replaced =
        update_current_chunks(ChunkSource::List(list.as_slice()), second, ChunkKind::Delta);
    assert_eq!(
        serde_json::to_value(&replaced).unwrap(),
        json!([
            {"type": "response.output_text.delta", "delta": "c"},
            {"type": "response.output_text.delta", "delta": "d"},
        ])
    );
    let audio_only = [audio_chunk()];
    assert!(
        update_current_chunks(
            ChunkSource::List(audio_only.as_slice()),
            replaced,
            ChunkKind::Delta
        )
        .is_none()
    );
    assert!(
        update_current_chunks(ChunkSource::Single(&audio_chunk()), None, ChunkKind::Delta)
            .is_none()
    );
    assert!(
        update_current_chunks(
            ChunkSource::Single(&value(r#"{"type": "response.done"}"#)),
            None,
            ChunkKind::Delta,
        )
        .is_none()
    );
    assert!(
        update_current_chunks(ChunkSource::Single(&value("5")), None, ChunkKind::Delta).is_none()
    );
    let with_scalar = [text_chunk("e"), value("5")];
    let filtered = update_current_chunks(
        ChunkSource::List(with_scalar.as_slice()),
        None,
        ChunkKind::Delta,
    );
    assert_eq!(
        serde_json::to_value(&filtered).unwrap(),
        json!([{"type": "response.output_text.delta", "delta": "e"}])
    );
}

#[test]
fn item_chunk_updates_follow_the_same_rule() {
    let message = item_done("item_1", "message");
    let call = item_done("item_2", "function_call");
    let first = update_current_chunks(ChunkSource::Single(&message), None, ChunkKind::Item);
    assert_eq!(
        serde_json::to_value(&first).unwrap()[0]["item"]["type"],
        "message"
    );
    let second = update_current_chunks(ChunkSource::Single(&call), first, ChunkKind::Item);
    assert_eq!(
        serde_json::to_value(&second)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let call_only = [item_done("item_3", "function_call")];
    let replaced = update_current_chunks(
        ChunkSource::List(call_only.as_slice()),
        second,
        ChunkKind::Item,
    );
    assert_eq!(
        serde_json::to_value(&replaced).unwrap()[0]["item"]["id"],
        "item_3"
    );
    let text_only = [text_chunk("x")];
    assert!(
        update_current_chunks(
            ChunkSource::List(text_only.as_slice()),
            replaced,
            ChunkKind::Item
        )
        .is_none()
    );
    assert!(
        update_current_chunks(ChunkSource::Single(&text_chunk("x")), None, ChunkKind::Item)
            .is_none()
    );
}

fn text_chunk(delta: &str) -> Json {
    serde_json::from_value(json!({"type": "response.output_text.delta", "delta": delta})).unwrap()
}

fn audio_chunk() -> Json {
    serde_json::from_value(json!({"type": "response.output_audio.delta", "delta": "QQ=="})).unwrap()
}

fn item_done(id: &str, item_type: &str) -> Json {
    serde_json::from_value(json!({
        "type": "response.output_item.done",
        "event_id": "event_1",
        "output_index": 0,
        "response_id": "resp_1",
        "item": {"id": id, "object": "realtime.item", "type": item_type, "status": "completed"},
    }))
    .unwrap()
}
