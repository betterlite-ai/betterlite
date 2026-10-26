#![cfg(test)]

use super::*;
use crate::models::finding::Role;
use rstest::rstest;
use std::fs;
use std::path::PathBuf;

#[test]
fn records_have_the_expected_shapes() {
    let observation: Observation =
        serde_json::from_str(r#"{"check_id":"c","summary":"ok"}"#).unwrap();
    assert_eq!(observation.kind, ObservationKind::Issue);
    let decision: Decision = serde_json::from_str(r#"{"action":"read"}"#).unwrap();
    assert_eq!(decision.page, 0);
}

#[test]
fn evidence_does_not_cross_omission_marker() {
    let part = TracePart {
        execution_id: String::from("e"),
        span_id: String::from("s"),
        parent_span_id: String::new(),
        name: String::from("n"),
        kind: String::from("k"),
        content: String::from("left\n[... content omitted ...]\nright"),
        truncated: false,
    };
    let evidence = Evidence {
        execution_id: String::from("e"),
        span_id: String::from("s"),
        quote: String::from("leftright"),
        role: Role::default(),
    };
    assert!(!evidence_valid(&evidence, &[part]));
}

#[test]
fn partition_rejects_zero_limit() {
    partition_items(&[1_i32], |_| 1_usize, 0).unwrap_err();
}

#[test]
fn default_concurrency_is_eight() {
    assert_eq!(ANALYSIS_CONCURRENCY, 8);
}

#[rstest]
fn replays_the_structured_response_corpus(
    #[files("fuzz/corpus/structured_response/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(serde_json::from_slice::<Decision>(&bytes));
    drop(serde_json::from_slice::<Examined>(&bytes));
}
