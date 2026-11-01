#![cfg(test)]

use crate::case::{LoadedCase, load_cases};
use crate::helpers::{HelperRun, run_case};
use std::path::Path;

#[test]
fn replays_the_s3_v2_params_case_corpus() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("cases/logging/s3-v2-params");
    let cases = load_cases(&[folder]).unwrap();
    assert!(cases.iter().all(|case| {
        let LoadedCase::Helper(helper) = case else {
            return false;
        };
        matches!(run_case(helper, false).unwrap(), HelperRun::Match)
    }));
}
