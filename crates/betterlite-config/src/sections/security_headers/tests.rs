#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::security_headers::Settings;

fn load(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("{}")
        .unwrap()
        .sections
        .security_headers
}

#[test]
fn hsts_is_off_when_the_variable_is_unset() {
    assert_eq!(load(&[]), Settings::default());
}

#[rstest]
#[case::exact("true")]
#[case::uppercase("TRUE")]
#[case::mixed_case("True")]
#[case::surrounding_whitespace("  true \n")]
fn hsts_is_on_for_true(#[case] value: &str) {
    assert!(load(&[("LITELLM_ENABLE_HSTS", value)]).enable_hsts);
}

#[rstest]
#[case::one("1")]
#[case::yes("yes")]
#[case::on("on")]
#[case::false_value("false")]
#[case::empty("")]
#[case::blank("   ")]
#[case::true_then_more("truex")]
#[case::other("enabled")]
fn hsts_stays_off_for_every_other_value(#[case] value: &str) {
    assert!(!load(&[("LITELLM_ENABLE_HSTS", value)]).enable_hsts);
}
