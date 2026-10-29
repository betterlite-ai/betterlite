#![cfg(test)]

use serde_json::value::RawValue;

use super::modality_token_sums;
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
