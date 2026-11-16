#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};

#[rstest]
fn matches_pinned_a2a_discover(
    #[base_dir = "../../.."]
    #[files("cases/helpers/a2a-discover/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).unwrap(),
        HelperRun::Match
    ));
}
