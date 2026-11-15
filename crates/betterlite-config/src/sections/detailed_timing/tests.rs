#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::sections::Section as _;
use crate::sections::detailed_timing::Raw;

fn settings(variables: &[(&str, &str)]) -> bool {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let raw: Raw = serde_saphyr::from_str("{}").unwrap();
    raw.convert(&environment).unwrap().enabled
}

#[rstest]
#[case::unset(&[], false)]
#[case::lowercase_true(&[("LITELLM_DETAILED_TIMING", "true")], true)]
#[case::uppercase_true(&[("LITELLM_DETAILED_TIMING", "TRUE")], true)]
#[case::mixed_case_true(&[("LITELLM_DETAILED_TIMING", "True")], true)]
#[case::one_is_not_true(&[("LITELLM_DETAILED_TIMING", "1")], false)]
#[case::yes_is_not_true(&[("LITELLM_DETAILED_TIMING", "yes")], false)]
#[case::empty_is_false(&[("LITELLM_DETAILED_TIMING", "")], false)]
#[case::padded_true_is_false(&[("LITELLM_DETAILED_TIMING", " true")], false)]
fn reads_the_detailed_timing_flag(#[case] variables: &[(&str, &str)], #[case] expected: bool) {
    assert_eq!(settings(variables), expected);
}
