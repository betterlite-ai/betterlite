#![cfg(test)]

use std::path::Path;

use betterlite_oracle_runner::case::load_helper_cases;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};

#[test]
fn replays_the_posthog_case_corpus() {
    let folder = Path::new(ORACLE_DIR).join("cases/logging/posthog");
    let cases = load_helper_cases(&[folder]).unwrap();
    assert!(cases.iter().all(|helper| {
        matches!(
            run_case(&[crate::GROUP], helper, false).unwrap(),
            HelperRun::Match
        )
    }));
}
