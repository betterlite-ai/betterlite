#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;

fn the_case_agrees(path: &Path) {
    let source = fs::read_to_string(path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).unwrap(),
        HelperRun::Match
    ));
}

#[rstest]
fn the_refresh_cases_agree(
    #[base_dir = "../../.."]
    #[files("cases/helpers/mcp-per-user-oauth-refresh/refresh-*.yaml")]
    path: PathBuf,
) {
    the_case_agrees(&path);
}

#[rstest]
fn the_lazy_cases_agree(
    #[base_dir = "../../.."]
    #[files("cases/helpers/mcp-per-user-oauth-refresh/lazy-*.yaml")]
    path: PathBuf,
) {
    the_case_agrees(&path);
}

#[rstest]
fn the_seed_cases_agree(
    #[base_dir = "../../.."]
    #[files("fuzz/corpus/helper_case/mcp-per-user-oauth-refresh-*.yaml")]
    path: PathBuf,
) {
    the_case_agrees(&path);
}
