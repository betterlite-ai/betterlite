#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;
use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> serde_json::Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/xai-usage")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

fn run_input(input: Value) -> Value {
    use crate::helpers::Helper as _;

    super::Op::decode_and_run(&input).unwrap()
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
}

#[rstest]
#[case::boolean_true(json!(true))]
#[case::boolean_false(json!(false))]
#[case::negative(json!(-5))]
#[case::float(json!(2.5))]
#[case::text(json!("100"))]
fn oracle_ticks_reject_non_integer_or_negative_counts(#[case] ticks: Value) {
    let result = run_input(json!({ "action": "ticks", "ticks": ticks }));
    assert_eq!(result["cost"], Value::Null);
    assert_eq!(result["restated_cost"], Value::Null);
}

#[test]
fn oracle_pricing_counts_reasoning_once() {
    let result = run_input(json!({
        "action": "cost",
        "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 35, "reasoning_tokens": 5},
    }));
    assert_eq!(result["prompt_cost"], "10");
    assert_eq!(result["completion_cost"], "25");
}

#[rstest]
#[case::malformed_text(json!("many"))]
#[case::fractional_text(json!("2.5"))]
#[case::negative(json!(-1))]
fn oracle_malformed_search_counts_attach_without_a_charge(#[case] calls: Value) {
    let result = run_input(json!({
        "action": "search",
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
        "details": {"web_search_calls": calls.clone()},
        "search_costs": {"search_context_size_medium": 0.01},
    }));
    assert_eq!(result["attached"], json!({"web_search_calls": calls}));
    assert_eq!(result["web_search_requests"], Value::Null);
    assert_eq!(result["search_cost"], "0");
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
