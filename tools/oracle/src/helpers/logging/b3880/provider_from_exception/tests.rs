#![cfg(test)]

use std::path::Path;

use betterlite_oracle_runner::case::load_helper_cases;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[rstest]
#[case::unset("provider-from-exception-unset.yaml")]
#[case::fallback("provider-from-exception-fallback.yaml")]
#[case::real("provider-from-exception-real.yaml")]
fn helper_case_matches_litellm(#[case] name: &str) {
    let path = Path::new(ORACLE_DIR)
        .join("cases/helpers/prometheus-failure")
        .join(name);
    let cases = load_helper_cases(&[path]).expect("the case loads");
    let Some(case) = cases.first() else {
        panic!("expected a helper case");
    };
    assert!(matches!(
        run_case(&[crate::GROUP], case, false).expect("the case runs"),
        HelperRun::Match
    ));
}
