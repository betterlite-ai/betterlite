#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;

#[rstest]
fn replays_the_http_case_corpus(
    #[base_dir = "../../.."]
    #[files("cases/guardrails/bedrock-apply/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    assert!(
        matches!(
            run_case(&[crate::GROUP], &case, false).unwrap(),
            HelperRun::Match
        ),
        "{} does not match LiteLLM",
        path.display()
    );
}
