#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::{HelperRun, run_case};

#[rstest]
fn helper_seeds_select_chirp_backend_policy(
    #[files("fuzz/corpus/helper_case/chirp_backend_policy*")] path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    validate_helper(&path, &case).unwrap();
    assert_eq!(case.call.operation, "chirp_backend_policy");
}

#[rstest]
fn matches_pinned_chirp_backend_policy(
    #[files("cases/helpers/chirp-backend-policy/*.yaml")] path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(matches!(run_case(&case, false).unwrap(), HelperRun::Match));
}
