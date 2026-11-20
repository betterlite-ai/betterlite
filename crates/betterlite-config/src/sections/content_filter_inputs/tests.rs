#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn loaded(environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load("")?;
    Ok(config.sections.content_filter_inputs)
}

#[test]
fn depth_defaults_to_hundred() {
    let settings = loaded(&[]).expect("the default loads");
    assert_eq!(settings.max_recurse_depth, 100);
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let alias = ConfigLoader { lookup }.load("DEFAULT_MAX_RECURSE_DEPTH: 5\n");
    assert!(
        matches!(alias, Err(ConfigError::Unsupported { .. })),
        "the setting has no YAML alias"
    );
}

#[rstest]
#[case::signed("+7", 7)]
#[case::whitespace(" 9 ", 9)]
#[case::negative("-4", -4)]
#[case::zero("0", 0)]
fn depth_reads_signed_and_padded_values(#[case] raw: &str, #[case] expected: i64) {
    let settings = loaded(&[("DEFAULT_MAX_RECURSE_DEPTH", raw)]).expect("a signed integer loads");
    assert_eq!(settings.max_recurse_depth, expected);
}

#[rstest]
#[case::text("many")]
#[case::fraction("1.5")]
#[case::overflow("9223372036854775808")]
fn depth_invalid_stops_startup(#[case] raw: &str) {
    let error = loaded(&[("DEFAULT_MAX_RECURSE_DEPTH", raw)]).expect_err("startup stops");
    assert!(
        matches!(error, ConfigError::Environment { .. }),
        "{error:?}"
    );
}

#[rstest]
#[case::empty("")]
#[case::blank("   ")]
fn depth_empty_is_unset(#[case] raw: &str) {
    let settings = loaded(&[("DEFAULT_MAX_RECURSE_DEPTH", raw)]).expect("empty counts as unset");
    assert_eq!(settings.max_recurse_depth, 100);
}
