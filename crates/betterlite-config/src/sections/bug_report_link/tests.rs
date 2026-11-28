#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::sections::Section as _;
use crate::sections::bug_report_link::{DISABLE_VARIABLE, Raw};

fn convert(variables: &[(&str, &str)]) -> Result<bool, String> {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let raw: Raw = serde_saphyr::from_str("{}").unwrap();
    raw.convert(&environment)
        .map(|settings| settings.disabled)
        .map_err(|error| error.to_string())
}

#[rstest]
#[case::unset(&[], false)]
#[case::empty(&[(DISABLE_VARIABLE, "")], false)]
#[case::blank(&[(DISABLE_VARIABLE, " ")], false)]
#[case::lowercase_true(&[(DISABLE_VARIABLE, "true")], true)]
#[case::capitalized_true(&[(DISABLE_VARIABLE, "True")], true)]
#[case::uppercase_true(&[(DISABLE_VARIABLE, "TRUE")], true)]
#[case::mixed_case_true(&[(DISABLE_VARIABLE, "tRuE")], true)]
#[case::one(&[(DISABLE_VARIABLE, "1")], true)]
#[case::yes(&[(DISABLE_VARIABLE, "yes")], true)]
#[case::on(&[(DISABLE_VARIABLE, "on")], true)]
#[case::lowercase_false(&[(DISABLE_VARIABLE, "false")], false)]
#[case::capitalized_false(&[(DISABLE_VARIABLE, "False")], false)]
#[case::zero(&[(DISABLE_VARIABLE, "0")], false)]
#[case::no(&[(DISABLE_VARIABLE, "no")], false)]
#[case::off(&[(DISABLE_VARIABLE, "off")], false)]
fn reads_the_disable_flag_with_the_shared_parser(
    #[case] variables: &[(&str, &str)],
    #[case] disabled: bool,
) {
    assert_eq!(convert(variables).unwrap(), disabled);
}

#[rstest]
#[case::word("maybe")]
#[case::number("two")]
fn a_value_the_parser_rejects_names_the_variable(#[case] value: &str) {
    let error = convert(&[(DISABLE_VARIABLE, value)]).unwrap_err();
    assert_eq!(
        error,
        format!("environment variable {DISABLE_VARIABLE} must be a bool")
    );
}
