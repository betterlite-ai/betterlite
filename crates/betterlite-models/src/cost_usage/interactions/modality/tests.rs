#![cfg(test)]

use rstest::rstest;
use serde_json::value::RawValue;

use super::{ModalityList, modality_token_sums};
use crate::cost_usage::error::CostUsageError;

fn raw(entries: &[&str]) -> Vec<Box<RawValue>> {
    entries
        .iter()
        .map(|entry| RawValue::from_string((*entry).to_owned()).expect("valid JSON"))
        .collect()
}

#[test]
fn document_maps_to_text_tokens() {
    let entries = raw(&[r#"{"modality":"document","tokens":5}"#]);
    let refs: Vec<&RawValue> = entries.iter().map(Box::as_ref).collect();
    let sums = modality_token_sums(&refs).expect("mapping entries");
    assert_eq!(sums.get("text_tokens"), Some(&5));
}

#[test]
fn unknown_modality_disappears() {
    let entries = raw(&[r#"{"modality":"smell","tokens":5}"#]);
    let refs: Vec<&RawValue> = entries.iter().map(Box::as_ref).collect();
    let sums = modality_token_sums(&refs).expect("mapping entries");
    assert!(sums.is_empty());
}

#[test]
fn bool_tokens_count_as_zero_or_one_and_strings_count_as_zero() {
    let entries = raw(&[
        r#"{"modality":"audio","tokens":true}"#,
        r#"{"modality":"audio","tokens":"3"}"#,
    ]);
    let refs: Vec<&RawValue> = entries.iter().map(Box::as_ref).collect();
    let sums = modality_token_sums(&refs).expect("mapping entries");
    assert_eq!(sums.get("audio_tokens"), Some(&1));
}

#[test]
fn non_mapping_entry_is_an_error() {
    let entries = raw(&["\"not an object\""]);
    let refs: Vec<&RawValue> = entries.iter().map(Box::as_ref).collect();
    let error = modality_token_sums(&refs).expect_err("non-mapping entry");
    assert_eq!(error, CostUsageError::ModalityEntryNotObject);
}

#[test]
fn array_entry_is_an_error_not_positional_fields() {
    let entries = raw(&[r#"["text",5]"#]);
    let refs: Vec<&RawValue> = entries.iter().map(Box::as_ref).collect();
    let error = modality_token_sums(&refs).expect_err("array entry");
    assert_eq!(error, CostUsageError::ModalityEntryNotObject);
}

#[test]
fn empty_array_entry_is_also_an_error() {
    let entries = raw(&["[]"]);
    let refs: Vec<&RawValue> = entries.iter().map(Box::as_ref).collect();
    let error = modality_token_sums(&refs).expect_err("empty array entry");
    assert_eq!(error, CostUsageError::ModalityEntryNotObject);
}

#[test]
fn single_entry_token_count_beyond_i64_is_a_counter_overflow() {
    let entries = raw(&[r#"{"modality":"text","tokens":9223372036854775808}"#]);
    let refs: Vec<&RawValue> = entries.iter().map(Box::as_ref).collect();
    let error = modality_token_sums(&refs).expect_err("counter overflow");
    assert_eq!(error, CostUsageError::CounterOverflow);
}

#[test]
fn modality_list_array_becomes_entries() {
    let list: ModalityList =
        serde_json::from_str(r#"[{"modality":"text","tokens":1}]"#).expect("valid JSON");
    assert_eq!(list.entries().len(), 1);
}

#[rstest]
#[case::false_value("false")]
#[case::zero("0")]
#[case::empty_string("\"\"")]
#[case::empty_object("{}")]
#[case::null("null")]
fn modality_list_falsy_shapes_are_empty(#[case] source: &str) {
    let list: ModalityList = serde_json::from_str(source).expect("valid JSON");
    assert!(list.entries().is_empty());
}

#[rstest]
#[case::true_value("true")]
#[case::one("1")]
#[case::non_empty_string("\"x\"")]
#[case::non_empty_object(r#"{"a":1}"#)]
#[case::large_number("1e400")]
fn modality_list_truthy_non_array_is_an_error(#[case] source: &str) {
    let list: ModalityList = serde_json::from_str(source).expect("valid JSON");
    assert!(matches!(list, ModalityList::NotArray), "{source}");
}

#[test]
fn sum_overflow_is_reported() {
    let entries = raw(&[
        &format!(r#"{{"modality":"text","tokens":{}}}"#, i64::MAX),
        r#"{"modality":"text","tokens":1}"#,
    ]);
    let refs: Vec<&RawValue> = entries.iter().map(Box::as_ref).collect();
    let error = modality_token_sums(&refs).expect_err("overflowing sum");
    assert_eq!(
        error,
        CostUsageError::ModalitySumOverflow {
            field: "text_tokens"
        }
    );
}
