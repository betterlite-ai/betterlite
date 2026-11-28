#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[rstest]
fn matches_the_pinned_auth_pricing(
    #[base_dir = "../../.."]
    #[files("cases/helpers/auth-pricing/*.yaml")]
    #[files("cases/auth/auth-pricing/*.yaml")]
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
fn helper_seed_selects_auth_pricing() {
    let seed = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case/auth-pricing-seed.yaml");
    let source = fs::read_to_string(seed).expect("seed file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(source.contains("auth_pricing:"));
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("seed runs"),
        HelperRun::Match
    ));
}
