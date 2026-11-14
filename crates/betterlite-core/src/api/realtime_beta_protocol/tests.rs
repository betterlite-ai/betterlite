#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::error::BetaProtocolError;
use super::events::{translate_event_to_beta, translate_item_content_types};
use super::headers::{
    HeaderField, HeaderPair, HeaderScope, client_sent_openai_beta_realtime_header,
};
use super::session::remap_beta_session_to_ga;

fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

#[derive(Clone, Copy)]
enum Encoding {
    Text,
    Bytes,
}

fn latin1(text: &str) -> Vec<u8> {
    text.chars()
        .map(|character| u8::try_from(u32::from(character)).unwrap())
        .collect()
}

fn header_pair(name: &str, value: &str, encoding: Encoding) -> HeaderPair {
    match encoding {
        Encoding::Text => (
            HeaderField::Text(name.to_owned()),
            HeaderField::Text(value.to_owned()),
        ),
        Encoding::Bytes => (
            HeaderField::Bytes(latin1(name)),
            HeaderField::Bytes(latin1(value)),
        ),
    }
}

#[rstest]
#[case::byte_header(Encoding::Bytes, Some(&[("OpenAI-Beta", "realtime=v1")]), true)]
#[case::string_header(Encoding::Text, Some(&[("OpenAI-Beta", "realtime=v1")]), true)]
#[case::uppercase_value(Encoding::Bytes, Some(&[("OpenAI-Beta", "REALTIME=V1")]), true)]
#[case::mixed_case_name(Encoding::Text, Some(&[("openai-BETA", "Realtime=V1")]), true)]
#[case::other_token_first(Encoding::Text, Some(&[("OpenAI-Beta", "other, realtime=v1")]), true)]
#[case::substring_v10(Encoding::Text, Some(&[("OpenAI-Beta", "realtime=v10")]), true)]
#[case::missing_header(Encoding::Text, Some(&[]), false)]
#[case::older_version(Encoding::Text, Some(&[("OpenAI-Beta", "realtime=v2")]), false)]
#[case::wrong_name(Encoding::Text, Some(&[("X-Beta", "realtime=v1")]), false)]
#[case::one_duplicate_matches(
    Encoding::Text,
    Some(&[("OpenAI-Beta", "realtime=v2"), ("OpenAI-Beta", "realtime=v1")]),
    true
)]
#[case::no_duplicate_matches(
    Encoding::Text,
    Some(&[("OpenAI-Beta", "realtime=v2"), ("OpenAI-Beta", "realtime=v3")]),
    false
)]
#[case::latin1_byte_after_the_token(
    Encoding::Bytes,
    Some(&[("OpenAI-Beta", "realtime=v1\u{ff}")]),
    true
)]
#[case::latin1_byte_in_the_name(
    Encoding::Bytes,
    Some(&[("OpenAI-Beta\u{ff}", "realtime=v1")]),
    false
)]
#[case::unreadable_scope(Encoding::Text, None, false)]
fn beta_header_detection(
    #[case] encoding: Encoding,
    #[case] pairs: Option<&[(&str, &str)]>,
    #[case] expected: bool,
) {
    let headers: Vec<HeaderPair> = match pairs {
        Some(pairs) => pairs
            .iter()
            .map(|(name, value)| header_pair(name, value, encoding))
            .collect(),
        None => Vec::new(),
    };
    let scope = HeaderScope { headers: &headers };
    let found = client_sent_openai_beta_realtime_header(pairs.map(|_| &scope));
    assert_eq!(found, expected);
}

#[test]
fn beta_session_audio_mapping() {
    let session = parse(
        r#"{
            "modalities": ["TEXT", "AUDIO"],
            "voice": "alloy",
            "input_audio_format": "pcm16",
            "output_audio_format": "g711_ulaw",
            "turn_detection": {"type": "server_vad"},
            "input_audio_transcription": {"model": "whisper-1"}
        }"#,
    );
    let expected = parse(
        r#"{
            "type": "realtime",
            "output_modalities": ["audio"],
            "audio": {
                "input": {
                    "format": {"type": "audio/pcm", "rate": 24000},
                    "turn_detection": {"type": "server_vad"},
                    "transcription": {"model": "whisper-1"}
                },
                "output": {
                    "voice": "alloy",
                    "format": {"type": "audio/G711-ulaw", "rate": 8000}
                }
            }
        }"#,
    );
    assert_eq!(remap_beta_session_to_ga(session).unwrap(), expected);
}

#[test]
fn beta_session_audio_mapping_alaw_and_passthrough_formats() {
    assert_eq!(
        remap_beta_session_to_ga(parse(
            r#"{"input_audio_format": "g711_alaw", "output_audio_format": "future-codec"}"#
        ))
        .unwrap(),
        parse(
            r#"{
                "type": "realtime",
                "audio": {
                    "input": {"format": {"type": "audio/G711-alaw", "rate": 8000}},
                    "output": {"format": "future-codec"}
                }
            }"#
        )
    );
    let object_format = parse(
        r#"{"input_audio_format": {"type": "audio/custom", "rate": 16000}, "voice": "alloy"}"#,
    );
    assert_eq!(
        remap_beta_session_to_ga(object_format).unwrap(),
        parse(
            r#"{
                "type": "realtime",
                "audio": {
                    "input": {"format": {"type": "audio/custom", "rate": 16000}},
                    "output": {"voice": "alloy"}
                }
            }"#
        )
    );
}

#[test]
fn beta_session_audio_mapping_modalities_become_output_modalities() {
    assert_eq!(
        remap_beta_session_to_ga(parse(r#"{"modalities": ["text"]}"#)).unwrap(),
        parse(r#"{"type": "realtime", "output_modalities": ["text"]}"#)
    );
    assert_eq!(
        remap_beta_session_to_ga(parse(
            r#"{"modalities": ["audio", "text"], "voice": "alloy"}"#
        ))
        .unwrap(),
        parse(
            r#"{
                "type": "realtime",
                "output_modalities": ["audio"],
                "audio": {"output": {"voice": "alloy"}}
            }"#
        )
    );
    assert_eq!(
        remap_beta_session_to_ga(parse(r#"{"modalities": []}"#)).unwrap(),
        parse(r#"{"type": "realtime"}"#)
    );
    assert_eq!(
        remap_beta_session_to_ga(parse(r#"{"modalities": null}"#)).unwrap(),
        parse(r#"{"type": "realtime"}"#)
    );
    assert_eq!(
        remap_beta_session_to_ga(parse(r#"{"modalities": ["unknown"]}"#)).unwrap(),
        parse(r#"{"type": "realtime"}"#)
    );
    assert_eq!(
        remap_beta_session_to_ga(parse(
            r#"{"modalities": ["audio"], "output_modalities": null}"#
        ))
        .unwrap(),
        parse(r#"{"type": "realtime", "output_modalities": null}"#)
    );
}

#[test]
fn beta_session_audio_mapping_keeps_type_and_unrelated_fields() {
    assert_eq!(
        remap_beta_session_to_ga(parse(
            r#"{"type": "session.update", "instructions": "hi", "tools": [{"type": "function"}], "future": {"deep": true}}"#
        ))
        .unwrap(),
        parse(
            r#"{"type": "session.update", "instructions": "hi", "tools": [{"type": "function"}], "future": {"deep": true}}"#
        )
    );
    assert_eq!(
        remap_beta_session_to_ga(parse(r#"{"type": null, "voice": "alloy"}"#)).unwrap(),
        parse(r#"{"type": null, "audio": {"output": {"voice": "alloy"}}}"#)
    );
}

#[test]
fn beta_session_nested_precedence() {
    let session = parse(
        r#"{
            "audio": {
                "input": {"format": {"type": "audio/old", "rate": 1}, "noise_reduction": {"type": "near_field"}},
                "output": {"voice": "old", "speed": 1.1}
            },
            "input_audio_format": "pcm16",
            "voice": "alloy"
        }"#,
    );
    let expected = parse(
        r#"{
            "type": "realtime",
            "audio": {
                "input": {
                    "format": {"type": "audio/pcm", "rate": 24000},
                    "noise_reduction": {"type": "near_field"}
                },
                "output": {"voice": "alloy", "speed": 1.1}
            }
        }"#,
    );
    assert_eq!(remap_beta_session_to_ga(session).unwrap(), expected);
}

#[test]
fn beta_session_nested_precedence_null_legacy_field_wins() {
    assert_eq!(
        remap_beta_session_to_ga(parse(
            r#"{"audio": {"input": {"turn_detection": {"type": "server_vad"}}}, "turn_detection": null}"#
        ))
        .unwrap(),
        parse(
            r#"{"type": "realtime", "audio": {"input": {"turn_detection": null}}}"#
        )
    );
}

#[test]
fn beta_session_nested_precedence_rejects_malformed_types() {
    assert!(matches!(
        remap_beta_session_to_ga(parse(r#"{"modalities": ["audio", 5]}"#)),
        Err(BetaProtocolError::Modalities)
    ));
    assert!(matches!(
        remap_beta_session_to_ga(parse(r#"{"modalities": "audio"}"#)),
        Err(BetaProtocolError::Modalities)
    ));
    assert!(matches!(
        remap_beta_session_to_ga(parse(r#"{"audio": "not-an-object", "voice": "alloy"}"#)),
        Err(BetaProtocolError::Audio)
    ));
    assert!(matches!(
        remap_beta_session_to_ga(parse("[]")),
        Err(BetaProtocolError::Session)
    ));
}

#[rstest]
#[case::item_added("conversation.item.added", "conversation.item.created")]
#[case::text_delta("response.output_text.delta", "response.text.delta")]
#[case::audio_delta("response.output_audio.delta", "response.audio.delta")]
#[case::transcript_delta(
    "response.output_audio_transcript.delta",
    "response.audio_transcript.delta"
)]
#[case::text_done("response.output_text.done", "response.text.done")]
#[case::audio_done("response.output_audio.done", "response.audio.done")]
#[case::transcript_done(
    "response.output_audio_transcript.done",
    "response.audio_transcript.done"
)]
fn ga_events_for_beta(#[case] ga_type: &str, #[case] beta_type: &str) {
    let event = parse(&format!(r#"{{"type": "{ga_type}", "event_id": "e1"}}"#));
    let expected = parse(&format!(r#"{{"type": "{beta_type}", "event_id": "e1"}}"#));
    assert_eq!(translate_event_to_beta(event).unwrap(), Some(expected));
}

#[test]
fn ga_events_for_beta_drops_conversation_item_done() {
    assert_eq!(
        translate_event_to_beta(parse(r#"{"type": "conversation.item.done"}"#)).unwrap(),
        None
    );
    assert_eq!(
        translate_event_to_beta(parse(
            r#"{"type": "response.done", "response": {"output": []}}"#
        ))
        .unwrap(),
        Some(parse(
            r#"{"type": "response.done", "response": {"output": []}}"#
        ))
    );
}

#[test]
fn ga_events_for_beta_translates_item_content() {
    let event = parse(
        r#"{
            "type": "conversation.item.added",
            "item": {
                "id": "i1",
                "content": [
                    {"type": "output_text", "text": "hi"},
                    {"type": "output_audio", "audio": "AAA"},
                    {"type": "input_audio", "audio": "BBB"},
                    "not-an-object"
                ]
            }
        }"#,
    );
    let expected = parse(
        r#"{
            "type": "conversation.item.created",
            "item": {
                "id": "i1",
                "content": [
                    {"type": "text", "text": "hi"},
                    {"type": "audio", "audio": "AAA"},
                    {"type": "input_audio", "audio": "BBB"},
                    "not-an-object"
                ]
            }
        }"#,
    );
    assert_eq!(translate_event_to_beta(event).unwrap(), Some(expected));
}

#[test]
fn ga_events_for_beta_translates_response_output_of_unknown_events() {
    let event = parse(
        r#"{
            "type": "response.future",
            "response": {
                "id": "r1",
                "output": [
                    {"type": "message", "content": [{"type": "output_text", "text": "hi"}]},
                    {"type": "output_audio", "audio": "AAA"},
                    7
                ]
            }
        }"#,
    );
    let expected = parse(
        r#"{
            "type": "response.future",
            "response": {
                "id": "r1",
                "output": [
                    {"type": "message", "content": [{"type": "text", "text": "hi"}]},
                    {"type": "audio", "audio": "AAA"},
                    7
                ]
            }
        }"#,
    );
    assert_eq!(translate_event_to_beta(event).unwrap(), Some(expected));
}

#[test]
fn ga_events_for_beta_keeps_events_without_nested_records() {
    let event = parse(r#"{"type": "response.created", "response_id": "r1"}"#);
    assert_eq!(translate_event_to_beta(event.clone()).unwrap(), Some(event));
    assert_eq!(
        translate_event_to_beta(parse(
            r#"{"type": "response.output_text.delta", "response": {"output": null}}"#
        ))
        .unwrap(),
        Some(parse(
            r#"{"type": "response.text.delta", "response": {"output": null}}"#
        ))
    );
}

#[test]
fn ga_events_for_beta_rejects_malformed_types() {
    assert!(matches!(
        translate_event_to_beta(parse(r#"{"type": 5}"#)),
        Err(BetaProtocolError::EventType)
    ));
    assert!(matches!(
        translate_event_to_beta(parse("[]")),
        Err(BetaProtocolError::Event)
    ));
    assert!(matches!(
        translate_event_to_beta(parse(r#"{"item": {"content": [{"type": 5}]}}"#)),
        Err(BetaProtocolError::ContentType)
    ));
}

#[test]
fn translate_item_content_types_leaves_items_without_content_lists() {
    assert_eq!(
        translate_item_content_types(&indexmap_of(r#"{"type": "message"}"#)).unwrap(),
        indexmap_of(r#"{"type": "message"}"#)
    );
    assert_eq!(
        translate_item_content_types(&indexmap_of(r#"{"content": "text"}"#)).unwrap(),
        indexmap_of(r#"{"content": "text"}"#)
    );
}

fn indexmap_of(text: &str) -> indexmap::IndexMap<String, Json> {
    let Json::Object(fields) = parse(text) else {
        panic!("the fixture must be an object");
    };
    fields
}
