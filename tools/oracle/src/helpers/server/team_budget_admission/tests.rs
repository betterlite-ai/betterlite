#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[rstest]
fn matches_the_pinned_team_budget_admission(
    #[base_dir = "../../.."]
    #[files("cases/helpers/team-budget-admission/*.yaml")]
    #[files("cases/auth/team-budget-admission/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(path).expect("case file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("case runs"),
        HelperRun::Match
    ));
}

#[test]
fn helper_seed_selects_team_budget_admission() {
    let seed =
        Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case/team-budget-admission-seed.yaml");
    let source = fs::read_to_string(seed).expect("seed file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(source.contains("team_budget_admission:"));
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("seed runs"),
        HelperRun::Match
    ));
}
