#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        clippy::too_many_lines,
        reason = "stream fixtures state provider JSON and wire bytes directly"
    )
)]

use serde_json::json;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_to_chat_stream::stream::{ParsedChatLine, get_model_response_iterator};
use crate::api::responses_to_chat_stream::text::repr_value;

use super::support::{GENERATED_ID, capture, fixed_clock, fixed_identifier, padding_value, stream};

#[test]
fn stream_identity_keeps_the_created_id_on_every_chunk() {
    let mut iterator = stream();
    let created = iterator
        .chunk_parser(&json!({"type": "response.created", "response": {"id": "resp_a"}}))
        .unwrap();
    assert_eq!(created.id, "resp_a");
    let delta = iterator
        .chunk_parser(&json!({"type": "response.output_text.delta", "delta": "hi"}))
        .unwrap();
    assert_eq!(delta.id, "resp_a");
    assert_eq!(delta.choices[0].delta.content.as_deref(), Some("hi"));
    let terminal = iterator
        .chunk_parser(&json!({"type": "response.completed", "response": {"id": "resp_b"}}))
        .unwrap();
    assert_eq!(terminal.id, "resp_a");
}

#[test]
fn stream_identity_keeps_the_first_generated_id() {
    let mut iterator = stream();
    let first = iterator
        .chunk_parser(&json!({"type": "response.output_text.delta", "delta": "a"}))
        .unwrap();
    assert_eq!(first.id, GENERATED_ID);
    let created = iterator
        .chunk_parser(&json!({"type": "response.created", "response": {"id": "resp_a"}}))
        .unwrap();
    assert_eq!(created.id, GENERATED_ID);
}

#[test]
fn stream_identity_is_never_shared_between_iterators() {
    let mut first = stream();
    let mut second = stream();
    let chunk = json!({"type": "response.created", "response": {"id": "resp_a"}});
    assert_eq!(first.chunk_parser(&chunk).unwrap().id, "resp_a");
    let other = second
        .chunk_parser(&json!({"type": "response.created", "response": {"id": "resp_b"}}))
        .unwrap();
    assert_eq!(other.id, "resp_b");
}

#[test]
fn stream_tier_is_remembered_and_overridden() {
    let mut iterator = stream();
    let created = iterator
        .chunk_parser(&json!({"type": "response.created", "response": {"id": "resp_a", "service_tier": "scale"}}))
        .unwrap();
    assert_eq!(created.service_tier.flatten().as_deref(), Some("scale"));
    let carried = iterator
        .chunk_parser(&json!({"type": "response.output_text.delta", "delta": "b"}))
        .unwrap();
    assert_eq!(carried.service_tier.flatten().as_deref(), Some("scale"));
    let reset = iterator
        .chunk_parser(
            &json!({"type": "response.created", "response": {"id": "resp_a", "service_tier": ""}}),
        )
        .unwrap();
    assert_eq!(reset.service_tier.flatten().as_deref(), Some("scale"));
    let terminal = iterator
        .chunk_parser(&json!({"type": "response.completed", "response": {"id": "resp_a", "service_tier": ""}}))
        .unwrap();
    assert_eq!(terminal.service_tier.flatten().as_deref(), Some(""));
}

#[test]
fn stream_tier_ignores_a_non_string_tier() {
    let mut iterator = stream();
    iterator
        .chunk_parser(
            &json!({"type": "response.created", "response": {"id": "resp_a", "service_tier": 7}}),
        )
        .unwrap();
    let carried = iterator
        .chunk_parser(&json!({"type": "response.output_text.delta", "delta": "b"}))
        .unwrap();
    assert_eq!(carried.service_tier, None);
}

#[test]
fn stream_tier_fills_a_passthrough_chunk() {
    let mut iterator = stream();
    iterator
        .chunk_parser(&json!({"type": "response.created", "response": {"id": "resp_a", "service_tier": "scale"}}))
        .unwrap();
    let overridden = iterator
        .chunk_parser(
            &json!({"object": "chat.completion.chunk", "service_tier": null, "choices": []}),
        )
        .unwrap();
    assert_eq!(overridden.service_tier.flatten().as_deref(), Some("scale"));
    let kept = iterator
        .chunk_parser(
            &json!({"object": "chat.completion.chunk", "service_tier": "", "choices": []}),
        )
        .unwrap();
    assert_eq!(kept.service_tier.flatten().as_deref(), Some(""));
    let absent = iterator
        .chunk_parser(&json!({"object": "chat.completion.chunk", "choices": []}))
        .unwrap();
    assert_eq!(
        absent.service_tier.as_ref().and_then(Option::as_deref),
        Some("scale")
    );
    let text = serde_json::to_string(&absent).unwrap();
    assert_eq!(text.matches("\"service_tier\"").count(), 1);
}

#[test]
fn stream_logs_each_translation_step() {
    let mut iterator = stream();
    let (created, lines) = capture(|| {
        iterator
            .chunk_parser(&json!({"type": "response.created", "response": {"id": "resp_a"}}))
            .unwrap()
    });
    // the response-id decoder logs its own debug line for an id that is not an encoded envelope
    let translated: Vec<String> = lines
        .into_iter()
        .filter(|line| line.starts_with("Chat provider: "))
        .collect();
    assert_eq!(
        translated,
        vec![
            "Chat provider: transform_streaming_response called with chunk: {'type': 'response.created', 'response': {'id': 'resp_a'}}".to_owned(),
            "Chat provider: Processing event type: response.created".to_owned(),
            "Chat provider: response.created -> {'type': 'response.created', 'response': {'id': 'resp_a'}}".to_owned(),
        ]
    );
    assert_eq!(created.id, "resp_a");

    let (_, passthrough) = capture(|| {
        iterator
            .chunk_parser(&json!({"object": "chat.completion.chunk", "choices": [{"index": 0}]}))
            .unwrap()
    });
    assert_eq!(passthrough.len(), 1);
    assert!(
        passthrough[0]
            .starts_with("Chat provider: transform_streaming_response called with chunk: ")
    );

    let (_, unhandled) = capture(|| {
        iterator
            .chunk_parser(&json!({"type": "response.failed"}))
            .unwrap()
    });
    assert_eq!(
        unhandled.last().unwrap(),
        "Chat provider: Unhandled event type 'response.failed', creating empty chunk"
    );

    let (_, conversation) = capture(|| {
        iterator
            .chunk_parser(&json!({"type": "conversation.item.created"}))
            .unwrap()
    });
    assert!(
        conversation.contains(
            &"Chat provider: Processing event type: conversation.item.created".to_owned()
        )
    );
}

#[test]
fn stream_parse_line_feeds_decoded_events_and_padding() {
    let mut iterator = stream();
    let padding = iterator
        .parse_line(b"event: response.created", &ApiLimits::default())
        .unwrap();
    match padding {
        ParsedChatLine::Padding(chunk) => {
            assert_eq!(serde_json::to_value(chunk).unwrap(), padding_value());
        }
        ParsedChatLine::Chunk(_) => panic!("an event line is no chunk"),
    }
    let line = br#"data: {"type": "response.output_text.delta", "delta": "hi"}"#;
    match iterator.parse_line(line, &ApiLimits::default()).unwrap() {
        ParsedChatLine::Chunk(chunk) => {
            assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"));
        }
        ParsedChatLine::Padding(_) => panic!("a data line is a chunk"),
    }
}

#[test]
fn stream_parse_line_reports_a_malformed_line() {
    let mut iterator = stream();
    let failure = iterator
        .parse_line(b"{", &ApiLimits::default())
        .unwrap_err();
    assert_eq!(
        failure.to_string(),
        "Expecting property name enclosed in double quotes: line 1 column 2 (char 1)"
    );
}

#[test]
fn repr_matches_python_text() {
    assert_eq!(repr_value(&json!(null)), "None");
    assert_eq!(repr_value(&json!(true)), "True");
    assert_eq!(repr_value(&json!(false)), "False");
    assert_eq!(repr_value(&json!(1.5)), "1.5");
    assert_eq!(repr_value(&json!("it's")), "'it\\'s'");
    assert_eq!(repr_value(&json!("a\\b\nc\rd\te")), "'a\\\\b\\nc\\rd\\te'");
    assert_eq!(repr_value(&json!(["a", 1])), "['a', 1]");
    assert_eq!(repr_value(&json!({"a": {"b": "c"}})), "{'a': {'b': 'c'}}");
}

#[test]
fn get_model_response_iterator_keeps_its_json_mode() {
    let iterator = get_model_response_iterator(Some(true), fixed_identifier, fixed_clock);
    assert_eq!(iterator.json_mode, Some(true));
    assert_eq!(iterator.chat_completion_id, None);
    assert_eq!(iterator.served_service_tier, None);
    assert!(iterator.tool_call_index_map.is_empty());
    let defaulted = get_model_response_iterator(None, fixed_identifier, fixed_clock);
    assert_eq!(defaulted.json_mode, None);
}
