#![cfg(test)]

use rstest::rstest;
use serde_json::value::RawValue;

use super::{GroundingToolCount, google_search_query_count};
use crate::cost_usage::error::CostUsageError;

fn raw(entries: &[&str]) -> Vec<Box<RawValue>> {
    entries
        .iter()
        .map(|entry| RawValue::from_string((*entry).to_owned()).expect("valid JSON"))
        .collect()
}

#[test]
fn sums_only_matching_type_entries() {
    let entries = raw(&[
        r#"{"type":"google_search","count":3}"#,
        r#"{"type":"other","count":9}"#,
        r#"{"type":"google_search","count":2}"#,
    ]);
    let refs: Vec<&RawValue> = entries.iter().map(Box::as_ref).collect();
    let total = google_search_query_count(&refs).expect("mapping entries");
    assert_eq!(total, 5);
}

#[test]
fn non_mapping_entries_are_silently_skipped() {
    let entries = raw(&[
        "\"not an object\"",
        "42",
        "[\"google_search\",3]",
        r#"{"type":"google_search","count":4}"#,
    ]);
    let refs: Vec<&RawValue> = entries.iter().map(Box::as_ref).collect();
    let total = google_search_query_count(&refs).expect("non-mapping entries are skipped");
    assert_eq!(total, 4);
}

#[test]
fn search_count_overflow_is_reported() {
    let entries = raw(&[
        &format!(r#"{{"type":"google_search","count":{}}}"#, i64::MAX),
        r#"{"type":"google_search","count":1}"#,
    ]);
    let refs: Vec<&RawValue> = entries.iter().map(Box::as_ref).collect();
    let error = google_search_query_count(&refs).expect_err("overflowing sum");
    assert_eq!(error, CostUsageError::SearchCountOverflow);
}

#[test]
fn single_count_beyond_i64_is_a_counter_overflow() {
    let entries = raw(&[r#"{"type":"google_search","count":9223372036854775808}"#]);
    let refs: Vec<&RawValue> = entries.iter().map(Box::as_ref).collect();
    let error = google_search_query_count(&refs).expect_err("counter overflow");
    assert_eq!(error, CostUsageError::CounterOverflow);
}

#[test]
fn grounding_tool_count_array_is_entries() {
    let count: GroundingToolCount =
        serde_json::from_str(r#"[{"type":"google_search"}]"#).expect("valid JSON");
    assert_eq!(count.entries().len(), 1);
}

#[rstest]
#[case::false_value("false")]
#[case::zero("0")]
#[case::empty_string("\"\"")]
#[case::empty_object("{}")]
#[case::null("null")]
#[case::non_empty_string("\"google_search\"")]
fn grounding_tool_count_non_array_shapes_are_empty(#[case] source: &str) {
    let count: GroundingToolCount = serde_json::from_str(source).expect("valid JSON");
    assert!(count.entries().is_empty());
}
