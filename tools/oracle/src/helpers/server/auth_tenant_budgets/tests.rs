#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[rstest]
fn matches_the_pinned_auth_tenant_budgets(
    #[base_dir = "../../.."]
    #[files("cases/helpers/auth-tenant-budgets/*.yaml")]
    #[files("cases/auth/auth-tenant-budgets/*.yaml")]
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
fn helper_seed_selects_auth_tenant_budgets() {
    let seed = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case/auth-tenant-budgets-seed.yaml");
    let source = fs::read_to_string(seed).expect("seed file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(source.contains("auth_tenant_budgets:"));
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("seed runs"),
        HelperRun::Match
    ));
}
