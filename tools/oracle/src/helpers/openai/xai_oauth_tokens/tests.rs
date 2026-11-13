#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::{HelperRun, run_case};

#[rstest]
fn helper_seeds_select_xai_oauth_tokens(
    #[files("fuzz/corpus/helper_case/xai_oauth_tokens*")] path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    validate_helper(&path, &case).unwrap();
    assert_eq!(case.call.operation, "xai_oauth_tokens");
}

#[rstest]
fn matches_pinned_xai_oauth_tokens(
    #[files("cases/helpers/xai-oauth-tokens/*.yaml")] path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(matches!(run_case(&case, false).unwrap(), HelperRun::Match));
}
