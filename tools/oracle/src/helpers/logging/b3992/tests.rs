#![cfg(test)]

use betterlite_oracle_runner::case::load_helper_cases;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use std::path::Path;

#[test]
fn replays_the_s3_v2_params_case_corpus() {
    let folder = Path::new(ORACLE_DIR).join("cases/logging/s3-v2-params");
    let cases = load_helper_cases(&[folder]).unwrap();
    assert!(cases.iter().all(|helper| {
        matches!(
            run_case(&[crate::GROUP], helper, false).unwrap(),
            HelperRun::Match
        )
    }));
}
