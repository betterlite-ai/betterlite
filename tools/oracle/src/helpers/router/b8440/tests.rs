#![cfg(test)]

use crate::helpers::GROUPS;
use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;
use std::fs;
use std::path::Path;

#[rstest]
#[case::targets("silent-experiment/targets")]
#[case::mirror_kwargs("silent-experiment/mirror-kwargs")]
#[case::excluded_filter("weighted-failover/excluded-filter")]
fn replays_b8440_against_litellm(#[case] name: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join(format!("cases/helpers/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
    let result = run_case(&GROUPS, &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}
