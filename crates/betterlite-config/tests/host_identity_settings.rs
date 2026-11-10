#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::host_identity_settings::Settings;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

fn settings(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("an empty config loads")
        .sections
        .host_identity_settings
}

fn revealed(key: Option<&SecretString>) -> Option<&str> {
    key.map(ExposeSecret::expose_secret)
}

fn resolved_key(key: &SecretString) -> String {
    key.expose_secret().to_owned()
}

#[test]
fn host_identity_settings_default_to_nothing() {
    let loaded = settings(&[]);
    assert_eq!(revealed(loaded.clarifai_api_key.as_ref()), None);
    assert_eq!(revealed(loaded.compactifai_api_key.as_ref()), None);
    assert_eq!(revealed(loaded.datarobot_api_token.as_ref()), None);
    assert_eq!(loaded.datarobot_endpoint, None);
}

#[rstest]
#[case::clarifai(&[("CLARIFAI_API_KEY", "sk-clarifai")], (Some("sk-clarifai"), None, None, None))]
#[case::compactifai(&[("COMPACTIFAI_API_KEY", "sk-compactifai")], (None, Some("sk-compactifai"), None, None))]
#[case::datarobot_token(&[("DATAROBOT_API_TOKEN", "datarobot-token")], (None, None, Some("datarobot-token"), None))]
#[case::datarobot_endpoint(
    &[("DATAROBOT_ENDPOINT", "https://tenant.example/api/v2")],
    (None, None, None, Some("https://tenant.example/api/v2"))
)]
fn loads_each_environment_key(
    #[case] environment: &[(&str, &str)],
    #[case] expected: (Option<&str>, Option<&str>, Option<&str>, Option<&str>),
) {
    let loaded = settings(environment);
    let (clarifai, compactifai, token, endpoint) = expected;
    assert_eq!(revealed(loaded.clarifai_api_key.as_ref()), clarifai);
    assert_eq!(revealed(loaded.compactifai_api_key.as_ref()), compactifai);
    assert_eq!(revealed(loaded.datarobot_api_token.as_ref()), token);
    assert_eq!(loaded.datarobot_endpoint.as_deref(), endpoint);
}

#[test]
fn empty_secret_environment_values_are_unset() {
    let loaded = settings(&[
        ("CLARIFAI_API_KEY", ""),
        ("COMPACTIFAI_API_KEY", ""),
        ("DATAROBOT_API_TOKEN", ""),
    ]);
    assert!(loaded.clarifai_api_key.is_none());
    assert!(loaded.compactifai_api_key.is_none());
    assert!(loaded.datarobot_api_token.is_none());
}

#[test]
fn empty_datarobot_endpoint_is_not_missing() {
    let loaded = settings(&[("DATAROBOT_ENDPOINT", "")]);
    assert_eq!(loaded.datarobot_endpoint.as_deref(), Some(""));
    assert!(settings(&[]).datarobot_endpoint.is_none());
}

const CLARIFAI_DEFAULT: &str = "https://api.clarifai.com/v2/ext/openai/v1";
const COMPACTIFAI_DEFAULT: &str = "https://api.compactif.ai/v1";
const DATAROBOT_DEFAULT: &str = "https://app.datarobot.com";

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

#[rstest]
#[case::explicit_wins(&[("CLARIFAI_API_KEY", "env-key")], Some("https://explicit.example"), Some("explicit-key"), "https://explicit.example", "explicit-key")]
#[case::absent_takes_environment(&[("CLARIFAI_API_KEY", "env-key")], None, None, CLARIFAI_DEFAULT, "env-key")]
#[case::empty_falls_through_to_environment(&[("CLARIFAI_API_KEY", "env-key")], Some(""), Some(""), CLARIFAI_DEFAULT, "env-key")]
#[case::nothing_anywhere(&[], None, None, CLARIFAI_DEFAULT, "")]
#[case::empty_without_environment(&[], Some(""), Some(""), CLARIFAI_DEFAULT, "")]
fn clarifai_resolution_follows_the_pinned_fallbacks(
    #[case] environment: &[(&str, &str)],
    #[case] explicit_base: Option<&str>,
    #[case] explicit_key: Option<&str>,
    #[case] base: &str,
    #[case] key: &str,
) {
    let explicit = explicit_key.map(secret);
    let (resolved_base, resolved) =
        settings(environment).resolve_clarifai(explicit_base, explicit.as_ref());
    assert_eq!(resolved_base, base);
    assert_eq!(resolved_key(&resolved), key);
}

#[rstest]
#[case::explicit_wins(&[("COMPACTIFAI_API_KEY", "env-key")], Some("https://explicit.example"), Some("explicit-key"), "https://explicit.example", "explicit-key")]
#[case::absent_takes_environment(&[("COMPACTIFAI_API_KEY", "env-key")], None, None, COMPACTIFAI_DEFAULT, "env-key")]
#[case::empty_falls_through_to_environment(&[("COMPACTIFAI_API_KEY", "env-key")], Some(""), Some(""), COMPACTIFAI_DEFAULT, "env-key")]
#[case::nothing_anywhere(&[], None, None, COMPACTIFAI_DEFAULT, "")]
fn compactifai_resolution_follows_the_pinned_fallbacks(
    #[case] environment: &[(&str, &str)],
    #[case] explicit_base: Option<&str>,
    #[case] explicit_key: Option<&str>,
    #[case] base: &str,
    #[case] key: &str,
) {
    let explicit = explicit_key.map(secret);
    let (resolved_base, resolved) =
        settings(environment).resolve_compactifai(explicit_base, explicit.as_ref());
    assert_eq!(resolved_base, base);
    assert_eq!(resolved_key(&resolved), key);
}

#[rstest]
#[case::nothing_set(&[], None, None, DATAROBOT_DEFAULT, "fake-api-key")]
#[case::environment_values(
    &[("DATAROBOT_ENDPOINT", "https://tenant.example"), ("DATAROBOT_API_TOKEN", "env-token")],
    None,
    None,
    "https://tenant.example",
    "env-token"
)]
#[case::explicit_values(
    &[("DATAROBOT_ENDPOINT", "https://tenant.example"), ("DATAROBOT_API_TOKEN", "env-token")],
    Some("https://explicit.example"),
    Some("explicit-key"),
    "https://explicit.example",
    "explicit-key"
)]
#[case::empty_explicit_takes_environment(
    &[("DATAROBOT_ENDPOINT", "https://tenant.example"), ("DATAROBOT_API_TOKEN", "env-token")],
    Some(""),
    Some(""),
    "https://tenant.example",
    "env-token"
)]
#[case::empty_endpoint_is_kept(&[("DATAROBOT_ENDPOINT", "")], None, None, "", "fake-api-key")]
#[case::empty_token_keeps_the_fake_key(&[("DATAROBOT_API_TOKEN", "")], None, None, DATAROBOT_DEFAULT, "fake-api-key")]
fn datarobot_resolution_keeps_empty_and_missing_apart(
    #[case] environment: &[(&str, &str)],
    #[case] explicit_base: Option<&str>,
    #[case] explicit_key: Option<&str>,
    #[case] base: &str,
    #[case] key: &str,
) {
    let explicit = explicit_key.map(secret);
    let (resolved_base, resolved) =
        settings(environment).resolve_datarobot(explicit_base, explicit.as_ref());
    assert_eq!(resolved_base, base);
    assert_eq!(resolved_key(&resolved), key);
}
