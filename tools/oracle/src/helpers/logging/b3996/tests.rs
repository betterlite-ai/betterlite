#![cfg(test)]

use std::path::Path;

use crate::case::{LoadedCase, load_cases};
use crate::helpers::{HelperRun, run_case};

#[test]
fn replays_corpus() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("cases/logging/gcs-mock");
    let cases = load_cases(&[folder]).unwrap();
    assert!(cases.iter().all(|case| {
        let LoadedCase::Helper(helper) = case else {
            return false;
        };
        matches!(run_case(helper, false).unwrap(), HelperRun::Match)
    }));
}
