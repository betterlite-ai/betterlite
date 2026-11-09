#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret;

use super::YouComSearchSettings;

fn load(environment: &[(&str, &str)]) -> YouComSearchSettings {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    YouComSearchSettings::load(&lookup)
}

#[test]
fn you_com_search_settings_default_to_unset() {
    let settings = load(&[]);
    assert!(settings.api_key.is_none());
    assert!(settings.api_base.is_none());
}

#[rstest]
#[case::key_and_base(
    &[("YOUCOM_API_KEY", "sk-test"), ("YOUCOM_API_BASE", "https://proxy.test")],
    Some("sk-test"),
    Some("https://proxy.test")
)]
#[case::empty_key_is_unset(&[("YOUCOM_API_KEY", "")], None, None)]
#[case::empty_base_is_unset(&[("YOUCOM_API_BASE", "")], None, None)]
#[case::unrelated_variables_are_ignored(&[("OTHER", "x")], None, None)]
fn you_com_search_settings_read_the_environment(
    #[case] environment: &[(&str, &str)],
    #[case] expected_key: Option<&str>,
    #[case] expected_base: Option<&str>,
) {
    let settings = load(environment);
    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        expected_key
    );
    assert_eq!(settings.api_base.as_deref(), expected_base);
}
