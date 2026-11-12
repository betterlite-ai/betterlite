#![cfg(test)]

use super::Settings;
use crate::loader::ConfigLoader;
use rstest::rstest;
use std::collections::BTreeMap;

fn strict_with(value: Option<&str>) -> bool {
    let lookup = value
        .map(|text| ("LITELLM_STRICT_GUARDRAIL_MODES".to_owned(), text.to_owned()))
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .unwrap()
        .sections
        .guardrail_modes
        .strict
}

#[rstest]
#[case::unset(None, true)]
#[case::upper_true(Some("TRUE"), true)]
#[case::lower_true(Some("true"), true)]
#[case::one(Some("1"), true)]
#[case::off(Some("off"), true)]
#[case::no(Some("no"), true)]
#[case::empty(Some(""), true)]
#[case::padded_false(Some(" False "), false)]
#[case::lower_false(Some("false"), false)]
#[case::upper_false(Some("FALSE"), false)]
fn modes_section_strict_reads_only_true_and_false(
    #[case] value: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(strict_with(value), expected);
}

#[test]
fn modes_section_default_is_strict() {
    assert_eq!(Settings::default(), Settings { strict: true });
}
