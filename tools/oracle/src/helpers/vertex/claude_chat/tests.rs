#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;

#[rstest]
fn helper_seeds_select_vertex_claude_chat(
    #[base_dir = "../../.."]
    #[files("fuzz/corpus/helper_case/vertex_claude_chat*")]
    path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    validate_helper(&path, &case).unwrap();
    assert_eq!(case.call.operation, "vertex_claude_chat");
}

#[rstest]
fn matches_pinned_vertex_claude_chat(
    #[base_dir = "../../.."]
    #[files("cases/helpers/vertex-claude-chat/*.yaml")]
    path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).unwrap(),
        HelperRun::Match
    ));
}
