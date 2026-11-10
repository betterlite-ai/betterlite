#![cfg(test)]

use betterlite_oracle::case::helper::HelperCase;
use betterlite_oracle::helpers::{HelperRun, run_case};
use rstest::rstest;
use std::{fs, path::PathBuf};

#[rstest]
fn matches_pinned_redaction(
    #[base_dir = "../../.."]
    #[files("cases/helpers/redaction/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    assert!(matches!(run_case(&case, false).unwrap(), HelperRun::Match));
}
