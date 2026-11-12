#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::Settings;

const DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH";

fn load(environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load("")?
        .sections
        .vertex_schema_shaping)
}

#[rstest]
#[case::absent(&[], 100)]
#[case::nondefault(&[(DEPTH_VARIABLE, "7")], 7)]
#[case::zero(&[(DEPTH_VARIABLE, "0")], 0)]
#[case::empty_is_unset(&[(DEPTH_VARIABLE, "")], 100)]
#[case::surrounding_spaces_trimmed(&[(DEPTH_VARIABLE, " 4 ")], 4)]
fn vertex_schema_settings_reads_the_recursion_depth(
    #[case] environment: &[(&str, &str)],
    #[case] expected: usize,
) {
    let settings = load(environment).expect("the depth loads");
    assert_eq!(settings.max_recurse_depth, expected);
}

#[test]
fn vertex_schema_settings_default_is_one_hundred() {
    assert_eq!(Settings::default().max_recurse_depth, 100);
}

#[rstest]
#[case::text("many")]
#[case::float("1.5")]
#[case::negative("-2")]
#[case::signs("--1")]
fn vertex_schema_settings_rejects_a_depth_that_is_not_a_count(#[case] value: &str) {
    let error = load(&[(DEPTH_VARIABLE, value)]).expect_err("a non-count depth fails");
    assert!(matches!(error, ConfigError::Environment { .. }));
    assert!(error.to_string().contains(DEPTH_VARIABLE));
}
