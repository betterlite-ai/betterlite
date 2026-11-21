#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use rstest::rstest;

use super::Op;

#[rstest]
fn helper_seeds_select_bedrock_services_s3_signing(
    #[base_dir = "../../.."]
    #[files("fuzz/corpus/helper_case/bedrock_services_s3_signing*")]
    path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    validate_helper(&path, &case).unwrap();
    assert_eq!(case.call.operation, "bedrock_services_s3_signing");
    Op::check(&case.call.input).unwrap();
}
