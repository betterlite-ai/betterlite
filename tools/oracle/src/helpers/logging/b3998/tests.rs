#![cfg(test)]

use std::path::Path;

use betterlite_oracle_runner::case::load_helper_cases;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};

#[test]
fn replays_corpus() {
    let folder = Path::new(ORACLE_DIR).join("cases/logging/gcs-bucket-logging");
    let cases = load_helper_cases(&[folder]).unwrap();
    assert!(cases.iter().all(|case| matches!(
        run_case(&[crate::GROUP], case, false).unwrap(),
        HelperRun::Match
    )));
}
