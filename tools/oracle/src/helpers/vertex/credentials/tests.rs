#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;

use super::Op;

#[rstest]
fn helper_seeds_select_vertex_credentials(
    #[files("fuzz/corpus/helper_case/vertex_credentials*.yaml")] path: PathBuf,
) {
    let source = fs::read_to_string(&path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    validate_helper(&path, &case).unwrap();
    assert_eq!(case.call.operation, "vertex_credentials");
}

#[test]
fn a_non_scenario_input_is_not_a_vertex_credentials_call() {
    assert!(Op::check(&serde_json::json!({"scenarios": 3_i32})).is_err());
}
