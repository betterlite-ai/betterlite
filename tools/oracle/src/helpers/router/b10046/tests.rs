#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;

use crate::helpers::GROUPS;

#[rstest]
#[case::both_limits_floored("io-both-limits-floored")]
#[case::input_only_minute_end("io-input-only-minute-end")]
#[case::output_only("io-output-only")]
#[case::zero_limit("io-zero-limit")]
#[case::no_limits("io-no-limits")]
#[case::merge_present_keys_win("merge-present-keys-win")]
fn replays_b10046_against_litellm(#[case] name: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root.join(format!("cases/helpers/remaining-usage-headers/{name}.yaml"));
    let source = fs::read_to_string(path).unwrap();
    let case = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
    let result = run_case(&GROUPS, &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}
