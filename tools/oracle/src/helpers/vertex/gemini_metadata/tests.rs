#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;
use serde_json::{Value, json};

use crate::case::HelperCase;

#[test]
fn helper_seeds_select_gemini_metadata() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gemini_metadata"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        seed.call.operation == "gemini_metadata"
            && seed
                .call
                .input
                .as_object()
                .is_some_and(|input| !input.is_empty())
    }));
}

#[rstest]
#[case::scalar_vs_list(json!({"call": "scalar_vs_list"}))]
#[case::nested_safety(json!({"call": "nested_safety"}))]
#[case::hidden_fallback(json!({"call": "hidden_fallback"}))]
#[case::empty_visible_shadow(json!({"call": "empty_visible_shadow"}))]
#[case::duplicate_chunks(json!({"call": "duplicate_chunks"}))]
#[case::first_nonweb_index(
    json!({"call": "first_nonweb_index", "grounding_metadata": []})
)]
#[case::citation_empty_title(
    json!({"call": "citation_empty_title", "content_text": "short", "grounding_metadata": []})
)]
fn every_call_mode_decodes(#[case] input: Value) {
    let decoded = serde_json::from_value::<super::ops::Input>(input);
    assert!(decoded.is_ok(), "{decoded:?}");
}

#[test]
fn an_unknown_call_mode_is_rejected() {
    let unknown = serde_json::from_value::<super::ops::Input>(json!({"call": "unknown_mode"}));
    assert!(unknown.unwrap_err().to_string().contains("unknown_mode"));
}

#[test]
fn helper_serializes_production_candidate_metadata() {
    let input: super::ops::Input = serde_json::from_value(json!({
        "call": "scalar_vs_list",
        "candidates": [
            {"groundingMetadata": {"webSearchQueries": ["alpha"]}},
            {"groundingMetadata": [{"webSearchQueries": ["beta"]}, {"webSearchQueries": ["gamma"]}]},
        ],
    }))
    .unwrap();
    assert_eq!(
        super::ops::run(&input),
        json!({
            "candidates": [
                {
                    "grounding_metadata": [{"webSearchQueries": ["alpha"]}],
                    "url_context_metadata": [],
                    "safety_ratings": [],
                    "citation_metadata": [],
                },
                {
                    "grounding_metadata": [{"webSearchQueries": ["beta"]}, {"webSearchQueries": ["gamma"]}],
                    "url_context_metadata": [],
                    "safety_ratings": [],
                    "citation_metadata": [],
                },
            ],
        })
    );
}
