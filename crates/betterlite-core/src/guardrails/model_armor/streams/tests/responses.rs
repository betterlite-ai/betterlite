use crate::guardrails::model_armor::streams::assembly::Assembly;
use crate::guardrails::model_armor::streams::responses::{
    content_to_scan, delta_field_texts, response_text,
};
use crate::guardrails::model_armor::streams::surface::Surface;
use crate::guardrails::model_armor::streams::wire::OutputParts;

use super::support::{FakeWire, Setup, chunk, value};

#[test]
fn responses_delta_joins_one_field_across_its_events() {
    let chunks = [
        chunk(
            r#"{"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "The "}"#,
        ),
        chunk(
            r#"{"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "answer"}"#,
        ),
    ];
    assert_eq!(delta_field_texts(&chunks), vec!["The answer".to_owned()]);
}

#[test]
fn responses_delta_keeps_separate_fields_apart() {
    let chunks = [
        chunk(
            r#"{"type": "response.reasoning_summary_text.delta", "item_id": "rs_1", "output_index": 1, "summary_index": 0, "delta": "thinking"}"#,
        ),
        chunk(
            r#"{"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "answer"}"#,
        ),
    ];
    assert_eq!(
        delta_field_texts(&chunks),
        vec!["thinking".to_owned(), "answer".to_owned()]
    );
}

#[test]
fn responses_delta_groups_by_content_index() {
    let chunks = [
        chunk(
            r#"{"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "a"}"#,
        ),
        chunk(
            r#"{"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 1, "delta": "b"}"#,
        ),
    ];
    assert_eq!(
        delta_field_texts(&chunks),
        vec!["a".to_owned(), "b".to_owned()]
    );
}

#[test]
fn responses_delta_groups_by_output_index_and_summary_index() {
    let chunks = [
        chunk(
            r#"{"type": "response.reasoning_summary_text.delta", "item_id": "rs_1", "output_index": 1, "summary_index": 0, "delta": "a"}"#,
        ),
        chunk(
            r#"{"type": "response.reasoning_summary_text.delta", "item_id": "rs_1", "output_index": 1, "summary_index": 1, "delta": "b"}"#,
        ),
        chunk(
            r#"{"type": "response.reasoning_summary_text.delta", "item_id": "rs_1", "output_index": 2, "summary_index": 0, "delta": "c"}"#,
        ),
    ];
    assert_eq!(
        delta_field_texts(&chunks),
        vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
    );
}

#[test]
fn responses_delta_groups_by_event_type() {
    let chunks = [
        chunk(
            r#"{"type": "response.output_text.delta", "item_id": "x", "output_index": 0, "delta": "a"}"#,
        ),
        chunk(
            r#"{"type": "response.refusal.delta", "item_id": "x", "output_index": 0, "delta": "b"}"#,
        ),
    ];
    assert_eq!(
        delta_field_texts(&chunks),
        vec!["a".to_owned(), "b".to_owned()]
    );
}

#[test]
fn responses_delta_ignores_events_outside_the_delta_types() {
    let chunks = [
        chunk(r#"{"type": "response.created", "delta": "x"}"#),
        chunk(r#"{"type": "response.output_text.done", "text": "y"}"#),
    ];
    assert!(delta_field_texts(&chunks).is_empty());
}

#[test]
fn responses_delta_ignores_a_non_text_delta() {
    let chunks = [chunk(
        r#"{"type": "response.output_text.delta", "delta": {"text": "x"}}"#,
    )];
    assert!(delta_field_texts(&chunks).is_empty());
}

#[test]
fn responses_delta_ignores_a_chunk_without_a_payload() {
    let chunks =
        [crate::guardrails::model_armor::streams::chunk::Chunk::Bytes(b"data: {}\n\n".to_vec())];
    assert!(delta_field_texts(&chunks).is_empty());
}

#[test]
fn responses_delta_fields_the_body_lacks_are_scanned_after_it() {
    let wire = FakeWire::new(Setup {
        parts: vec![OutputParts {
            texts: vec!["The answer".to_owned()],
            encoded_calls: Vec::new(),
        }],
        ..Setup::default()
    });
    let body = Assembly::Responses(value(r#"{"output": [{"index": 0}]}"#));
    let chunks = [
        chunk(
            r#"{"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "The "}"#,
        ),
        chunk(
            r#"{"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "answer"}"#,
        ),
        chunk(
            r#"{"type": "response.reasoning_summary_text.delta", "item_id": "rs_1", "output_index": 1, "summary_index": 0, "delta": "thinking"}"#,
        ),
    ];
    let content =
        content_to_scan(&wire, Surface::Responses, &body, &chunks).expect("the content reads");
    assert_eq!(content, "The answer\nthinking");
}

#[test]
fn responses_delta_fields_join_the_scan_without_a_leading_newline() {
    let wire = FakeWire::new(Setup::default());
    let body = Assembly::Responses(value(r#"{"output": []}"#));
    let chunks = [chunk(
        r#"{"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "The secret"}"#,
    )];
    let content =
        content_to_scan(&wire, Surface::Responses, &body, &chunks).expect("the content reads");
    assert_eq!(content, "The secret");
}

#[test]
fn responses_delta_fields_already_in_the_body_are_not_repeated() {
    let wire = FakeWire::new(Setup {
        parts: vec![OutputParts {
            texts: vec!["The answer".to_owned(), "thinking".to_owned()],
            encoded_calls: Vec::new(),
        }],
        ..Setup::default()
    });
    let body = Assembly::Responses(value(r#"{"output": [{"index": 0}]}"#));
    let chunks = [
        chunk(
            r#"{"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "The "}"#,
        ),
        chunk(
            r#"{"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "answer"}"#,
        ),
    ];
    let content =
        content_to_scan(&wire, Surface::Responses, &body, &chunks).expect("the content reads");
    assert_eq!(content, "The answerthinking");
}

#[test]
fn responses_delta_tool_arguments_are_scanned_when_the_body_lacks_them() {
    let wire = FakeWire::new(Setup {
        parts: vec![OutputParts {
            texts: vec!["The answer".to_owned()],
            encoded_calls: Vec::new(),
        }],
        ..Setup::default()
    });
    let body = Assembly::Responses(value(r#"{"output": [{"index": 0}]}"#));
    let chunks = [
        chunk(
            r#"{"type": "response.function_call_arguments.delta", "item_id": "fc_1", "output_index": 1, "delta": "{\"a\":"}"#,
        ),
        chunk(
            r#"{"type": "response.function_call_arguments.delta", "item_id": "fc_1", "output_index": 1, "delta": "1}"}"#,
        ),
    ];
    let content =
        content_to_scan(&wire, Surface::Responses, &body, &chunks).expect("the content reads");
    assert_eq!(content, "The answer\n{\"a\":1}");
}

#[test]
fn content_to_scan_reads_chat_message_text_through_the_wire() {
    let wire = FakeWire::new(Setup {
        model_text: Some("The secret".to_owned()),
        ..Setup::default()
    });
    let body = Assembly::Model(value(r#"{"choices": []}"#));
    let content =
        content_to_scan(&wire, Surface::ChatCompletions, &body, &[]).expect("the content reads");
    assert_eq!(content, "The secret");
}

#[test]
fn content_to_scan_is_empty_for_a_completion() {
    let wire = FakeWire::new(Setup {
        model_text: Some("ignored".to_owned()),
        ..Setup::default()
    });
    let body = Assembly::Completion(value(r#"{"choices": []}"#));
    let content =
        content_to_scan(&wire, Surface::ChatCompletions, &body, &[]).expect("the content reads");
    assert_eq!(content, "");
}

#[test]
fn content_to_scan_surfaces_an_unreadable_model_text() {
    let wire = FakeWire::new(Setup::default());
    let body = Assembly::Model(value(r#"{"choices": []}"#));
    assert!(content_to_scan(&wire, Surface::ChatCompletions, &body, &[]).is_err());
}

#[test]
fn response_text_joins_texts_then_encoded_calls_of_every_item() {
    let wire = FakeWire::new(Setup {
        parts: vec![
            OutputParts {
                texts: vec!["one".to_owned()],
                encoded_calls: vec!["{\"a\":1}".to_owned()],
            },
            OutputParts {
                texts: vec!["two".to_owned()],
                encoded_calls: Vec::new(),
            },
        ],
        ..Setup::default()
    });
    let body = value(r#"{"output": [{"index": 0}, {"index": 1}]}"#);
    assert_eq!(response_text(&wire, &body), "onetwo{\"a\":1}");
}

#[test]
fn response_text_is_empty_without_an_output_array() {
    let wire = FakeWire::new(Setup::default());
    assert_eq!(response_text(&wire, &value(r#"{"id": "resp_1"}"#)), "");
}
