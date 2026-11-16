#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;

const DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH";

// the Copilot vision walk reuses the shared recursion setting instead of reading the variable itself
#[rstest]
#[case::absent(&[], 100)]
#[case::nondefault(&[(DEPTH_VARIABLE, "7")], 7)]
#[case::zero(&[(DEPTH_VARIABLE, "0")], 0)]
#[case::empty_is_unset(&[(DEPTH_VARIABLE, "")], 100)]
fn copilot_vision_depth_settings_load_the_shared_recursion_depth(
    #[case] environment: &[(&str, &str)],
    #[case] expected: i64,
) {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load("").expect("the config loads");
    assert_eq!(
        config.sections.vertex_schema_shaping.max_recurse_depth,
        expected
    );
}
