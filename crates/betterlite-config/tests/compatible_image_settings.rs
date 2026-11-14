#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::compatible_image_credentials::{
    AIML_DEFAULT_API_BASE, COMETAPI_DEFAULT_API_BASE, Host, Settings,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

fn settings(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("an empty config loads")
        .sections
        .compatible_image_credentials
}

fn shown(settings: &Settings) -> (Option<&str>, Option<&str>, Option<&str>, Option<&str>) {
    (
        settings.aiml_api_base.as_deref(),
        settings
            .aiml_api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        settings.cometapi_api_base.as_deref(),
        settings
            .cometapi_api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
    )
}

#[test]
fn every_alias_is_absent_by_default() {
    assert_eq!(shown(&settings(&[])), (None, None, None, None));
}

#[rstest]
#[case::aiml_api_key(&[("AIML_API_KEY", "primary")], (None, Some("primary"), None, None))]
#[case::aimlapi_key(&[("AIMLAPI_KEY", "alternative")], (None, Some("alternative"), None, None))]
#[case::aiml_primary_wins(&[("AIML_API_KEY", "primary"), ("AIMLAPI_KEY", "alternative")], (None, Some("primary"), None, None))]
#[case::empty_aiml_primary(&[("AIML_API_KEY", ""), ("AIMLAPI_KEY", "alternative")], (None, Some("alternative"), None, None))]
#[case::cometapi_key(&[("COMETAPI_KEY", "primary")], (None, None, None, Some("primary")))]
#[case::cometapi_api_key(&[("COMETAPI_API_KEY", "alternative")], (None, None, None, Some("alternative")))]
#[case::cometapi_primary_wins(&[("COMETAPI_KEY", "primary"), ("COMETAPI_API_KEY", "alternative")], (None, None, None, Some("primary")))]
#[case::empty_cometapi_primary(&[("COMETAPI_KEY", ""), ("COMETAPI_API_KEY", "alternative")], (None, None, None, Some("alternative")))]
fn key_alias_chains(
    #[case] environment: &[(&str, &str)],
    #[case] expected: (Option<&str>, Option<&str>, Option<&str>, Option<&str>),
) {
    assert_eq!(shown(&settings(environment)), expected);
}

#[rstest]
#[case::aiml_base(&[("AIML_API_BASE", "https://aiml.example")], (Some("https://aiml.example"), None, None, None))]
#[case::cometapi_base_url(&[("COMETAPI_BASE_URL", "https://first.example")], (None, None, Some("https://first.example"), None))]
#[case::cometapi_api_base(&[("COMETAPI_API_BASE", "https://second.example")], (None, None, Some("https://second.example"), None))]
#[case::cometapi_base_url_wins(&[("COMETAPI_BASE_URL", "https://first.example"), ("COMETAPI_API_BASE", "https://second.example")], (None, None, Some("https://first.example"), None))]
#[case::empty_cometapi_base_url(&[("COMETAPI_BASE_URL", ""), ("COMETAPI_API_BASE", "https://second.example")], (None, None, Some("https://second.example"), None))]
#[case::empty_aiml_base(&[("AIML_API_BASE", "")], (None, None, None, None))]
fn base_alias_chains(
    #[case] environment: &[(&str, &str)],
    #[case] expected: (Option<&str>, Option<&str>, Option<&str>, Option<&str>),
) {
    assert_eq!(shown(&settings(environment)), expected);
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

#[rstest]
#[case::aiml_explicit_wins(Host::Aiml, &[("AIML_API_BASE", "https://env.example"), ("AIML_API_KEY", "env-key")], Some("https://explicit.example"), Some("explicit-key"), "https://explicit.example", Some("explicit-key"))]
#[case::aiml_absent_takes_environment(Host::Aiml, &[("AIML_API_BASE", "https://env.example"), ("AIML_API_KEY", "env-key")], None, None, "https://env.example", Some("env-key"))]
#[case::aiml_defaults(Host::Aiml, &[], None, None, AIML_DEFAULT_API_BASE, None)]
#[case::aiml_empty_falls_through(Host::Aiml, &[("AIML_API_BASE", "https://env.example"), ("AIML_API_KEY", "env-key")], Some(""), Some(""), "https://env.example", Some("env-key"))]
#[case::cometapi_explicit_wins(Host::CometApi, &[("COMETAPI_BASE_URL", "https://env.example"), ("COMETAPI_KEY", "env-key")], Some("https://explicit.example"), Some("explicit-key"), "https://explicit.example", Some("explicit-key"))]
#[case::cometapi_defaults(Host::CometApi, &[], None, None, COMETAPI_DEFAULT_API_BASE, None)]
#[case::cometapi_empty_falls_through(Host::CometApi, &[("COMETAPI_API_BASE", "https://env.example"), ("COMETAPI_API_KEY", "env-key")], Some(""), Some(""), "https://env.example", Some("env-key"))]
fn resolution_takes_the_explicit_value_first(
    #[case] host: Host,
    #[case] environment: &[(&str, &str)],
    #[case] api_base: Option<&str>,
    #[case] api_key: Option<&str>,
    #[case] expected_base: &str,
    #[case] expected_key: Option<&str>,
) {
    let supplied = api_key.map(secret);
    let (base, key) = settings(environment).resolve(host, api_base, supplied.as_ref());
    assert_eq!(base, expected_base);
    let resolved = key.as_ref().map(|secret| secret.expose_secret().to_owned());
    assert_eq!(resolved.as_deref(), expected_key);
}
