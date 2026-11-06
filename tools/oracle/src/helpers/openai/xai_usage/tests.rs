#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> serde_json::Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/xai-usage")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_xai_usage() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("xai_usage"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok() && case.call.operation == "xai_usage"
    }));
}

#[test]
fn oracle_ticks_divide_by_ten_billion() {
    let result = run_case("ticks-and-booleans.yaml");
    assert_eq!(result["cost"], "1.25");
    assert_eq!(result["restated_cost"], "1.25");
}

#[test]
fn oracle_reported_zero_suppresses_the_generic_price() {
    let result = run_case("reported-zero.yaml");
    assert_eq!(result["prompt_cost"], "0");
    assert_eq!(result["completion_cost"], "0");
    assert!(result["delegate"].is_null());
}

#[test]
fn oracle_reasoning_folds_once() {
    let result = run_case("fold-once-and-preserve-larger-total.yaml");
    let expected = serde_json::json!({
        "prompt_tokens": 10_u32,
        "completion_tokens": 25_u32,
        "total_tokens": 35_u32,
    });
    assert_eq!(result["once"], expected);
    assert_eq!(result["twice"], expected);
}

#[test]
fn oracle_larger_total_leaves_reasoning_raw() {
    let result = run_case("raw-reasoning-larger-total.yaml");
    let expected = serde_json::json!({
        "prompt_tokens": 10_u32,
        "completion_tokens": 20_u32,
        "total_tokens": 50_u32,
    });
    assert_eq!(result["once"], expected);
    assert_eq!(result["twice"], expected);
}

#[test]
fn oracle_numeric_text_counts_as_search_calls() {
    let result = run_case("search-count-conversion.yaml");
    assert_eq!(result["web_search_requests"], 3_u32);
    assert_eq!(result["search_cost"], "0.03");
}

#[test]
fn oracle_search_rate_tries_medium_then_low_then_high() {
    let result = run_case("medium-low-high-rate.yaml");
    assert_eq!(result["web_search_requests"], 2_u32);
    assert_eq!(result["search_cost"], "0.04");
}
