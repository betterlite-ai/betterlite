#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::case::HelperCase;
use crate::helpers::{HelperRun, run_case};

#[rstest]
fn matches_pinned_legacy_otel_policy(
    #[files("cases/helpers/legacy-otel-policy/*.yaml")] path: PathBuf,
) {
    let source = fs::read_to_string(path).expect("case file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(matches!(
        run_case(&case, false).expect("case runs"),
        HelperRun::Match
    ));
}
