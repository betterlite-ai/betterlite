#![cfg(test)]

use betterlite_oracle_runner::case::load_helper_cases;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use std::path::Path;

#[test]
fn replays_the_langfuse_otel_case_corpora() {
    let root = Path::new(ORACLE_DIR).join("cases");
    let folders = [
        root.join("logging/langfuse-otel"),
        root.join("helpers/langfuse-otel"),
    ];
    let cases = load_helper_cases(&folders).unwrap();
    assert!(!cases.is_empty());
    assert!(cases.iter().all(|helper| {
        matches!(
            run_case(&[crate::GROUP], helper, false).unwrap(),
            HelperRun::Match
        )
    }));
}
