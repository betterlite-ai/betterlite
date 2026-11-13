#![cfg(test)]

mod cases;
mod fixture;

use std::fs;
use std::path::Path;

use rstest::rstest;

use crate::stream_events::decode_stream_tool_arguments;

#[rstest]
#[case::message_start_role(cases::message_start_role)]
#[case::text_delta(cases::text_delta)]
#[case::tool_use_and_argument_delta(cases::tool_use_and_argument_delta)]
#[case::thinking_and_signature_events(cases::thinking_and_signature_events)]
#[case::citation_event(cases::citation_event)]
#[case::json_mode_chunks(cases::json_mode_chunks)]
#[case::ping_no_content(cases::ping_no_content)]
#[case::error_event(cases::error_event)]
#[case::content_start_reset(cases::content_start_reset)]
#[case::web_search_stream_lifecycle(cases::web_search_stream_lifecycle)]
#[case::compaction_and_container_events(cases::compaction_and_container_events)]
#[case::accumulated_json_values(cases::accumulated_json_values)]
fn stream_events_cases(#[case] scenario: fn()) {
    scenario();
}

fn corpus_outcomes() -> Vec<(String, bool)> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_stream_events");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let source = fs::read_to_string(&path).unwrap();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            (name, decode_stream_tool_arguments(&source).is_ok())
        })
        .collect()
}

#[test]
fn replays_the_anthropic_stream_events_corpus() {
    let outcomes = corpus_outcomes();
    assert!(!outcomes.is_empty());
    assert!(
        outcomes
            .iter()
            .any(|(name, ok)| name.starts_with("valid") && *ok)
    );
    assert!(outcomes.iter().any(|(name, ok)| {
        (name.starts_with("malformed") || name.starts_with("truncated")) && !ok
    }));
    assert!(!outcomes.iter().any(|(name, ok)| {
        (name.starts_with("malformed") || name.starts_with("truncated")) && *ok
    }));
    assert!(outcomes.iter().any(|(name, ok)| {
        (name.starts_with("boundary-empty") || name.starts_with("boundary-whitespace")) && !ok
    }));
    assert!(
        outcomes
            .iter()
            .any(|(name, ok)| name.starts_with("boundary-nested") && *ok)
    );
    assert!(
        outcomes
            .iter()
            .any(|(name, ok)| name.starts_with("boundary-large-number") && *ok)
    );
}
