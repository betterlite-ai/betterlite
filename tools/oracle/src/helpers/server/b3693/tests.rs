#![cfg(test)]

use std::path::Path;

use crate::case::{LoadedCase, load_cases};
use crate::helpers::{HelperRun, run_case};

#[test]
fn replays_pattern_matching_oracle_cases() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("cases/guardrails/pattern-matching");
    let cases = load_cases(&[folder]).unwrap();
    let selected: Vec<_> = cases
        .iter()
        .filter_map(|case| match case {
            LoadedCase::Helper(helper) if helper.call.operation == "b3693" => Some(helper),
            LoadedCase::Helper(_) | LoadedCase::Http(_) | LoadedCase::Scenario(_) => None,
        })
        .collect();
    assert!(!selected.is_empty());
    assert!(
        selected
            .iter()
            .all(|case| matches!(run_case(case, false).unwrap(), HelperRun::Match))
    );
}
