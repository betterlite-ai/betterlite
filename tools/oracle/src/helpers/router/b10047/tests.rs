#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;

use crate::helpers::GROUPS;

#[rstest]
#[case::every_occurrence("model-access-groups/every-occurrence")]
#[case::alias_rows_first("model-access-groups/alias-rows-first")]
#[case::exact_group("model-access-groups/exact-group")]
#[case::for_model_name("model-access-groups/for-model-name")]
#[case::team_lookup("model-access-groups/team-lookup")]
#[case::wildcard_request_name("model-access-groups/wildcard-request-name")]
fn replays_b10047_against_litellm(#[case] name: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join(format!("cases/helpers/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
    let result = run_case(&GROUPS, &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}
