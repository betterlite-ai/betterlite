#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[rstest]
fn matches_the_pinned_organization_route_policy(
    #[base_dir = "../../.."]
    #[files("cases/helpers/organization-route-policy/*.yaml")]
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
fn helper_seed_selects_organization_route_policy() {
    let seed =
        Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case/organization-route-policy-seed.yaml");
    let source = fs::read_to_string(seed).expect("seed file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(source.contains("organization_route_policy:"));
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("seed runs"),
        HelperRun::Match
    ));
}
