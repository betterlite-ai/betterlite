#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret;

use super::Settings;

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::nothing_set(&[], None, "https://api.hconeai.com")]
#[case::key_only(&[("HELICONE_API_KEY", "sk-helicone")], Some("sk-helicone"), "https://api.hconeai.com")]
#[case::base_override(&[("HELICONE_API_KEY", "k"), ("HELICONE_API_BASE", "http://localhost:8787")], Some("k"), "http://localhost:8787")]
#[case::empty_base_is_unset(&[("HELICONE_API_BASE", "")], None, "https://api.hconeai.com")]
#[case::empty_key_is_unset(&[("HELICONE_API_KEY", "")], None, "https://api.hconeai.com")]
#[case::one_trailing_slash_dropped(&[("HELICONE_API_BASE", "https://gateway.example/")], None, "https://gateway.example")]
#[case::only_one_slash_dropped(&[("HELICONE_API_BASE", "https://gateway.example//")], None, "https://gateway.example/")]
fn initialization_reads_the_key_and_base(
    #[case] variables: &[(&str, &str)],
    #[case] key: Option<&str>,
    #[case] base: &str,
) {
    let settings = Settings::from_environment(&environment(variables));
    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        key
    );
    assert_eq!(settings.api_base, base);
}

#[test]
fn initialization_debug_output_hides_the_key() {
    let settings =
        Settings::from_environment(&environment(&[("HELICONE_API_KEY", "sk-secret-value")]));
    assert!(!format!("{settings:?}").contains("sk-secret-value"));
}
