#![cfg(test)]

use std::path::PathBuf;

use rstest::rstest;

use betterlite_oracle_runner::case::load_helper_cases;
use betterlite_oracle_runner::helpers::{HelperRun, run_helper_cases};

#[rstest]
fn replays_the_cold_storage_key_cases(
    #[base_dir = "../../.."]
    #[files("cases/helpers/cold-storage-key/*.yaml")]
    path: PathBuf,
) {
    let cases = load_helper_cases(&[path]).unwrap();
    assert!(matches!(
        run_helper_cases(&[crate::GROUP], &cases, false).unwrap(),
        HelperRun::Match
    ));
}
