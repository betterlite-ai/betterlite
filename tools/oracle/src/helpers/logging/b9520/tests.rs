#![cfg(test)]

use std::path::Path;

use betterlite_oracle_runner::case::load_helper_cases;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};

#[test]
fn replays_the_cold_storage_key_cases() {
    let folder = Path::new(ORACLE_DIR).join("cases/helpers/cold-storage-key");
    let cases = load_helper_cases(&[folder]).unwrap();
    assert!(cases.iter().all(|helper| {
        matches!(
            run_case(&[crate::GROUP], helper, false).unwrap(),
            HelperRun::Match
        )
    }));
}
