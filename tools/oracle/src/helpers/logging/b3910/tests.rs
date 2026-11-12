#![cfg(test)]

use crate::case::{LoadedCase, load_cases};
use crate::helpers::{HelperRun, run_case};
use std::path::Path;

#[test]
fn replays_the_langfuse_otel_case_corpora() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("cases");
    let folders = [
        root.join("logging/langfuse-otel"),
        root.join("helpers/langfuse-otel"),
    ];
    let cases = load_cases(&folders).unwrap();
    assert!(!cases.is_empty());
    assert!(cases.iter().all(|case| {
        let LoadedCase::Helper(helper) = case else {
            return false;
        };
        matches!(run_case(helper, false).unwrap(), HelperRun::Match)
    }));
}
