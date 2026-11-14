#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret;

use crate::types::LiteLlmParams;

const DEFAULT_API_BASE: &str = "https://api.soniox.com";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn soniox_record_settings_default_to_an_unset_key() {
    let (api_key, api_base) = LiteLlmParams::soniox_record_settings(&lookup(&[]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}

#[test]
fn soniox_record_settings_read_the_environment() {
    let (api_key, api_base) = LiteLlmParams::soniox_record_settings(&lookup(&[
        ("SONIOX_API_KEY", "server-key"),
        ("SONIOX_API_BASE", "https://proxy.test"),
    ]));

    assert_eq!(
        api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("server-key")
    );
    assert_eq!(api_base, "https://proxy.test");
}

#[rstest]
#[case::empty_base_is_unset("SONIOX_API_BASE", "", None, DEFAULT_API_BASE)]
#[case::empty_key_is_kept("SONIOX_API_KEY", "", Some(""), DEFAULT_API_BASE)]
#[case::true_key_is_unset("SONIOX_API_KEY", "true", None, DEFAULT_API_BASE)]
#[case::false_base_is_unset("SONIOX_API_BASE", " False ", None, DEFAULT_API_BASE)]
#[case::other_text_key_is_kept("SONIOX_API_KEY", "yes", Some("yes"), DEFAULT_API_BASE)]
#[case::other_text_base_is_kept("SONIOX_API_BASE", "no", None, "no")]
fn soniox_record_settings_treat_empty_values(
    #[case] name: &str,
    #[case] value: &str,
    #[case] expected_key: Option<&str>,
    #[case] expected_base: &str,
) {
    let (api_key, api_base) = LiteLlmParams::soniox_record_settings(&lookup(&[(name, value)]));

    assert_eq!(
        api_key.as_ref().map(ExposeSecret::expose_secret),
        expected_key
    );
    assert_eq!(api_base, expected_base);
}

#[rstest]
#[case::one_slash("https://proxy.test/", "https://proxy.test")]
#[case::several_slashes("https://proxy.test///", "https://proxy.test")]
#[case::path_kept("https://proxy.test/api/", "https://proxy.test/api")]
#[case::only_slashes("///", "")]
fn soniox_record_settings_strip_trailing_slashes(#[case] value: &str, #[case] expected: &str) {
    let (_, api_base) =
        LiteLlmParams::soniox_record_settings(&lookup(&[("SONIOX_API_BASE", value)]));

    assert_eq!(api_base, expected);
}
