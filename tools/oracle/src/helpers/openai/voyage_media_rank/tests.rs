#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;

#[test]
fn helper_seeds_select_voyage_media_rank() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("voyage_media_rank"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "voyage_media_rank"
    }));
}

fn input_of(file: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/voyage-media-rank")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    case.call.input
}

#[test]
fn a_non_two_hundred_reply_is_a_top_level_error() {
    let error = super::Op::decode_and_run(&input_of("top-k-url-and-status.yaml")).unwrap_err();
    assert_eq!(error.status, Some(201));
}

#[test]
fn successful_rerank_calls_log_the_post_call_and_keep_the_request() {
    let value = super::Op::decode_and_run(&input_of("content-object-and-option-overlay.yaml"))
        .expect("overlay case");
    let rerank = value["rerank"].as_array().expect("rerank calls");
    assert_eq!(rerank[0]["post_call"], true);
    assert_eq!(rerank[0]["request"]["value"]["top_k"], 3);
    assert_eq!(rerank[0]["request"]["value"]["return_documents"], false);
    assert_eq!(
        rerank[0]["url"], "https://example.test/v1/rerank",
        "a trailing slash is trimmed before the suffix"
    );
    assert!(rerank[0]["request"]["value"].get("rank_fields").is_none());
}

#[test]
fn falsey_ids_use_the_injected_uuid() {
    let value = super::Op::decode_and_run(&input_of("document-id-and-token-cost.yaml"))
        .expect("document case");
    let rerank = value["rerank"].as_array().expect("rerank calls");
    assert_eq!(
        rerank[0]["response"]["value"]["id"],
        "00000000-0000-4000-8000-000000000001"
    );
    assert_eq!(
        rerank[0]["response"]["value"]["results"][0]["document"],
        serde_json::json!({"text": "doc"})
    );
    assert_eq!(rerank[1]["response"]["value"]["id"], "given");
}

#[test]
fn multimodal_failures_keep_the_value_error_status() {
    let value = super::Op::decode_and_run(&input_of("image-url-null-empty-and-data-prefix.yaml"))
        .expect("image case");
    let multimodal = value["multimodal"].as_array().expect("multimodal calls");
    assert_eq!(multimodal[0]["request"]["ok"], false);
    assert_eq!(multimodal[0]["request"]["status"], serde_json::Value::Null);
    assert_eq!(
        multimodal[2]["request"]["value"]["inputs"][0]["content"][0]["type"],
        "image_base64"
    );
}
