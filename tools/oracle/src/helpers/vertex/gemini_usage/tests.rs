#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::json;

use crate::case::HelperCase;

#[test]
fn helper_seeds_select_gemini_usage() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gemini_usage"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        seed.call.operation == "gemini_usage"
            && seed
                .call
                .input
                .as_object()
                .is_some_and(|input| !input.is_empty())
    }));
}

#[test]
fn an_unknown_input_field_is_rejected() {
    let unknown = serde_json::from_value::<super::ops::Input>(json!({"usageMetadata": {}}));
    assert!(unknown.unwrap_err().to_string().contains("usageMetadata"));
}

#[test]
fn a_missing_usage_metadata_reports_the_usage_error() {
    let input: super::ops::Input = serde_json::from_value(json!({})).unwrap();
    let failure = super::ops::run(&input).unwrap_err();
    assert!(failure.message.contains("usageMetadata"));
}

#[test]
fn the_operation_reaches_the_production_accounting() {
    let input: super::ops::Input = serde_json::from_value(json!({
        "usage_metadata": {
            "promptTokenCount": 10_i64,
            "candidatesTokenCount": 5_i64,
            "toolUsePromptTokenCount": 3_i64,
            "totalTokenCount": 18_i64,
            "thoughtsTokenCount": 4_i64,
        },
    }))
    .unwrap();
    let expected = json!({
        "prompt_tokens": 13_i64,
        "completion_tokens": 5_i64,
        "total_tokens": 18_i64,
        "prompt_tokens_details": {
            "text_tokens": null,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null,
            "cached_tokens": null,
            "tool_use_tokens": 3_i64,
            "web_search_requests": null,
            "google_maps_grounding_requests": null,
        },
        "completion_tokens_details": {
            "text_tokens": 5_i64,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null,
            "reasoning_tokens": 4_i64,
        },
    });
    assert_eq!(super::ops::run(&input).unwrap(), expected);
}
