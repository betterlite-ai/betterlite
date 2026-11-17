#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};

#[rstest]
fn helper_seeds_select_cloud_storage(
    #[base_dir = "../../.."]
    #[files("fuzz/corpus/helper_case/cloud_storage*")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    validate_helper(&path, &case).unwrap();
    assert_eq!(case.call.operation, "cloud_storage");
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).unwrap(),
        HelperRun::Match
    ));
}
