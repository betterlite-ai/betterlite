#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};

use super::Op;

#[rstest]
fn helper_seeds_select_vertex_aws_federation(
    #[base_dir = "../../.."]
    #[files("fuzz/corpus/helper_case/vertex_aws_federation*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    validate_helper(&path, &case).unwrap();
    assert_eq!(case.call.operation, "vertex_aws_federation");
}

#[test]
fn an_aws_field_of_the_wrong_type_is_not_a_vertex_aws_federation_input() {
    let input = serde_json::json!({"aws_region_name": 3_i32});
    assert!(Op::check(&input).is_err());
}
