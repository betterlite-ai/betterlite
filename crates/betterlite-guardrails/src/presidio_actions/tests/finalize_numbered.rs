#![cfg(test)]

use indexmap::IndexMap;

use super::shared::{item, request_data, tokens};
use crate::presidio_actions::anonymize::finalize_numbered_tokens;
use crate::presidio_actions::error::PresidioActionsError;

#[test]
fn numbers_entities_left_to_right_and_stores_the_original_text() {
    let detections = [item("PERSON", 6, 11), item("PERSON", 16, 19)];
    let mut data = request_data("{}");
    let mut counts = IndexMap::new();
    let text = finalize_numbered_tokens(
        "Hello Alice and Bob",
        &detections,
        Some(&mut data),
        &mut counts,
    )
    .unwrap();
    assert_eq!(text, "Hello <PERSON_1> and <PERSON_2>");
    let stored = tokens(&data);
    assert_eq!(
        stored.keys().cloned().collect::<Vec<_>>(),
        vec!["<PERSON_2>".to_owned(), "<PERSON_1>".to_owned()]
    );
    assert_eq!(stored.get("<PERSON_1>").map(String::as_str), Some("Alice"));
    assert_eq!(stored.get("<PERSON_2>").map(String::as_str), Some("Bob"));
    assert_eq!(counts.get("PERSON"), Some(&2));
}

#[test]
fn spans_are_character_offsets_not_byte_offsets() {
    let detections = [item("PERSON", 6, 11), item("PERSON", 16, 19)];
    let mut data = request_data("{}");
    let mut counts = IndexMap::new();
    let text = finalize_numbered_tokens(
        "h\u{e9}llo Alice and Bob",
        &detections,
        Some(&mut data),
        &mut counts,
    )
    .unwrap();
    assert_eq!(text, "h\u{e9}llo <PERSON_1> and <PERSON_2>");
    assert_eq!(
        tokens(&data).get("<PERSON_1>").map(String::as_str),
        Some("Alice")
    );
}

#[test]
fn existing_request_metadata_and_tokens_are_kept() {
    let mut data = request_data(
        r#"{"metadata":{"litellm_call_id":"call-1","pii_tokens":{"<OLD_1>":"prior"}}}"#,
    );
    let mut counts = IndexMap::new();
    finalize_numbered_tokens(
        "Alice",
        &[item("PERSON", 0, 5)],
        Some(&mut data),
        &mut counts,
    )
    .unwrap();
    let stored = tokens(&data);
    assert_eq!(stored.get("<OLD_1>").map(String::as_str), Some("prior"));
    assert_eq!(stored.get("<PERSON_1>").map(String::as_str), Some("Alice"));
    let metadata = data.get("metadata").unwrap();
    assert!(metadata.get().contains("call-1"));
}

#[test]
fn a_missing_request_data_still_builds_the_numbered_text() {
    let mut counts = IndexMap::new();
    let text = finalize_numbered_tokens("Hello Alice", &[item("PERSON", 6, 11)], None, &mut counts)
        .unwrap();
    assert_eq!(text, "Hello <PERSON_1>");
}

#[test]
fn a_repeated_span_takes_the_last_sequence_number() {
    let detections = [item("PERSON", 0, 5), item("LOCATION", 0, 5)];
    let mut counts = IndexMap::new();
    let text = finalize_numbered_tokens("Alice lives", &detections, None, &mut counts).unwrap();
    assert_eq!(text, "<PERSON_2>TION_2> lives");
    assert_eq!(counts.get("PERSON"), Some(&1));
    assert_eq!(counts.get("LOCATION"), Some(&1));
}

#[test]
fn overlapping_spans_store_the_unmodified_input_slices() {
    let detections = [item("PERSON", 0, 6), item("LOCATION", 3, 9)];
    let mut data = request_data("{}");
    let mut counts = IndexMap::new();
    let text =
        finalize_numbered_tokens("abcdefghij", &detections, Some(&mut data), &mut counts).unwrap();
    assert_eq!(text, "<PERSON_1>CATION_2>j");
    let stored = tokens(&data);
    assert_eq!(stored.get("<PERSON_1>").map(String::as_str), Some("abcdef"));
    assert_eq!(
        stored.get("<LOCATION_2>").map(String::as_str),
        Some("defghi")
    );
}

#[test]
fn a_detection_without_a_start_is_an_error() {
    let mut detection = item("PERSON", 0, 5);
    detection.start = None;
    let mut counts = IndexMap::new();
    let error = finalize_numbered_tokens("Alice", &[detection], None, &mut counts).unwrap_err();
    assert!(matches!(
        error,
        PresidioActionsError::MissingSpan { field: "start" }
    ));
}

#[test]
fn a_span_past_the_end_of_the_text_is_an_error() {
    let mut counts = IndexMap::new();
    let error =
        finalize_numbered_tokens("Alice", &[item("PERSON", 0, 99)], None, &mut counts).unwrap_err();
    assert!(matches!(
        error,
        PresidioActionsError::SpanOutOfBounds { start: 0, end: 99 }
    ));
}

#[test]
fn a_detection_without_an_entity_type_is_an_error() {
    let mut detection = item("PERSON", 0, 5);
    detection.entity_type = None;
    let mut counts = IndexMap::new();
    let error = finalize_numbered_tokens("Alice", &[detection], None, &mut counts).unwrap_err();
    assert!(matches!(error, PresidioActionsError::MissingEntityType));
}
