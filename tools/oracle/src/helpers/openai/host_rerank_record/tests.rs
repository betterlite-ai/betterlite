#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::{Helper as _, HelperResult};

const FIXED_ID: &str = "00000000-0000-4000-8000-000000000001";

fn run_case(file: &str) -> HelperResult {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/host-rerank-record")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input)
}

fn answer(file: &str) -> Value {
    run_case(file).unwrap()
}

#[test]
fn helper_seeds_select_host_rerank_record() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("host_rerank_record"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert_eq!(seeds.len(), 2);
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "host_rerank_record"
            && super::Op::decode_and_run(&case.call.input).is_ok()
    }));
}

#[test]
fn jina_reads_the_dictionary_and_not_the_formal_arguments() {
    let result = answer("jina-formals-versus-dictionary.yaml");
    assert_eq!(
        result["mapped"],
        json!({"top_n": 1, "return_documents": true})
    );
    assert_eq!(
        result["request"],
        json!({"model": "jina-reranker-v2", "top_n": 1, "return_documents": true})
    );
    assert_eq!(
        result["headers"],
        json!({
            "accept": "application/json",
            "content-type": "application/json",
            "authorization": "Bearer jk",
        })
    );
    assert_eq!(result["url"], "https://api.jina.ai/v1/rerank");
}

#[test]
fn an_empty_base_is_a_relative_path() {
    let result = answer("path-replacement-and-empty-base.yaml");
    assert_eq!(result["url"], "/v1/rerank");
    assert_eq!(result["request"]["query"], "q");
}

#[test]
fn a_base_path_is_replaced() {
    let result = answer("jina-path-replacement.yaml");
    assert_eq!(result["url"], "https://x.test/v1/rerank");
}

#[test]
fn a_jina_status_is_a_plain_error_with_the_body() {
    let error = run_case("generic-status-and-json-errors.yaml").unwrap_err();
    assert_eq!(error.status, None);
    assert_eq!(error.message, "upstream boom");
}

#[test]
fn an_infinity_body_that_is_not_json_carries_the_upstream_status() {
    let error = run_case("infinity-json-error-keeps-status.yaml").unwrap_err();
    assert_eq!(error.status, Some(502));
    assert!(error.message.contains("bad gateway"));
}

#[test]
fn an_empty_document_is_left_out_and_the_counters_are_read() {
    let result = answer("missing-results-and-empty-document.yaml");
    let response = &result["response"];
    assert_eq!(response["id"], "r1");
    assert!(response["results"][0].get("document").is_none());
    assert_eq!(response["results"][1]["document"], json!({"text": "b"}));
    assert_eq!(
        response["meta"]["tokens"],
        json!({"input_tokens": 3, "output_tokens": 2})
    );
}

#[test]
fn missing_infinity_results_are_an_empty_list_with_a_fresh_id() {
    let result = answer("infinity-missing-results.yaml");
    assert_eq!(result["response"]["results"], json!([]));
    assert_eq!(result["response"]["id"], FIXED_ID);
    assert_eq!(result["url"], "https://i.test/rerank");
}

#[test]
fn the_output_counter_is_not_clamped_at_zero() {
    let result = answer("negative-output-and-token-cost.yaml");
    assert_eq!(
        result["response"]["meta"]["tokens"],
        json!({"input_tokens": 10, "output_tokens": -6})
    );
    assert_eq!(result["cost"], Value::Null);
}

#[test]
fn jina_prices_the_total_tokens_and_the_output_is_free() {
    let result = answer("jina-token-cost.yaml");
    assert_eq!(result["cost"], json!([2.0, 0.0]));
    assert_eq!(result["response"]["id"], FIXED_ID);
    assert_eq!(
        result["response"]["results"][0]["document"],
        json!({"text": "a"})
    );
    assert_eq!(result["hook"].as_array().map(Vec::len), Some(1));
}
