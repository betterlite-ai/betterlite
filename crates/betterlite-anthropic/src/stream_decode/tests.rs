#![cfg(test)]

mod fixture;
mod scenarios;

use std::fs;
use std::path::Path;

use rstest::rstest;

use crate::stream_decode::decode_anthropic_event;

use fixture::converter;

#[rstest]
#[case::case_1_split_event_data(scenarios::split_event_data)]
#[case::case_2_split_unicode(scenarios::split_unicode)]
#[case::case_3_multi_line_sse(scenarios::multi_line_sse)]
#[case::case_4_done_marker(scenarios::done_marker)]
#[case::case_5_invalid_json_event(scenarios::invalid_json_event)]
#[case::case_6_native_error_event(scenarios::native_error_event)]
#[case::case_7_truncated_final_event(scenarios::truncated_final_event)]
#[case::case_8_passthrough_chunk_conversion(scenarios::passthrough_chunk_conversion)]
fn stream_decode_cases(#[case] scenario: fn()) {
    scenario();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Value,
    Pending,
    Failed,
}

fn corpus_outcomes() -> Vec<(String, Category)> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_stream_decode");
    let mut outcomes = Vec::new();
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let payload = fs::read(&path).unwrap();
        let category = match decode_anthropic_event(&mut converter(), &payload) {
            Ok(Some(_)) => Category::Value,
            Ok(None) => Category::Pending,
            Err(_) => Category::Failed,
        };
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        outcomes.push((name, category));
    }
    outcomes
}

fn expected_category(name: &str) -> Option<Category> {
    if name.starts_with("valid") {
        return Some(Category::Value);
    }
    if name.starts_with("truncated") {
        return Some(Category::Pending);
    }
    if name.starts_with("malformed") {
        return Some(Category::Failed);
    }
    None
}

#[test]
fn replays_the_anthropic_stream_decode_corpus() {
    let outcomes = corpus_outcomes();
    assert!(!outcomes.is_empty());
    let mismatched = outcomes
        .iter()
        .filter(|(name, category)| {
            expected_category(name).is_some_and(|expected| expected != *category)
        })
        .collect::<Vec<_>>();
    assert!(mismatched.is_empty(), "{mismatched:?}");
    assert!(
        outcomes
            .iter()
            .any(|(name, category)| name.starts_with("boundary") && *category == Category::Value)
    );
    assert!(
        outcomes
            .iter()
            .any(|(name, category)| name.starts_with("boundary") && *category == Category::Pending)
    );
    assert!(
        outcomes
            .iter()
            .any(|(name, category)| name.starts_with("boundary") && *category == Category::Failed)
    );
}
