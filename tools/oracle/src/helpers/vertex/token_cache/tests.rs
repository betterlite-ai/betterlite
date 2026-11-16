#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};

use super::Op;

#[rstest]
fn helper_seeds_select_vertex_token_cache(
    #[base_dir = "../../.."]
    #[files("fuzz/corpus/helper_case/vertex_token_cache*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    validate_helper(&path, &case).unwrap();
    assert_eq!(case.call.operation, "vertex_token_cache");
}

#[test]
fn a_non_scenario_input_is_not_a_vertex_token_cache_call() {
    assert!(Op::check(&serde_json::json!({"scenarios": 3_i32})).is_err());
}
