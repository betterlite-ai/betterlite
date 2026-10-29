#![cfg(test)]

use serde_json::value::RawValue;

use super::google_search_query_count;
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
