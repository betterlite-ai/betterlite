#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;

#[rstest]
fn matches_pinned_legacy_lifecycle(
    #[base_dir = "../../.."]
    #[files("cases/helpers/legacy_lifecycle/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(path).expect("case file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("case runs"),
        HelperRun::Match
    ));
}
