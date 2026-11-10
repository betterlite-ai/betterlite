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

#[derive(Clone, Copy)]
enum Host {
    Clarifai,
    Compactifai,
    Datarobot,
}

fn resolve(
    host: Host,
    settings: &Settings,
    explicit_base: Option<&str>,
    explicit_key: Option<&SecretString>,
) -> (String, SecretString) {
    match host {
        Host::Clarifai => settings.resolve_clarifai(explicit_base, explicit_key),
        Host::Compactifai => settings.resolve_compactifai(explicit_base, explicit_key),
        Host::Datarobot => settings.resolve_datarobot(explicit_base, explicit_key),
    }
}

#[rstest]
#[case::clarifai_explicit_wins(Host::Clarifai, &[("CLARIFAI_API_KEY", "env-key")], Some("https://explicit.example"), Some("explicit-key"), ("https://explicit.example", "explicit-key"))]
#[case::clarifai_absent_takes_environment(Host::Clarifai, &[("CLARIFAI_API_KEY", "env-key")], None, None, (CLARIFAI_DEFAULT, "env-key"))]
#[case::clarifai_empty_falls_through(Host::Clarifai, &[("CLARIFAI_API_KEY", "env-key")], Some(""), Some(""), (CLARIFAI_DEFAULT, "env-key"))]
#[case::clarifai_nothing_anywhere(Host::Clarifai, &[], None, None, (CLARIFAI_DEFAULT, ""))]
#[case::clarifai_empty_without_environment(Host::Clarifai, &[], Some(""), Some(""), (CLARIFAI_DEFAULT, ""))]
#[case::compactifai_explicit_wins(Host::Compactifai, &[("COMPACTIFAI_API_KEY", "env-key")], Some("https://explicit.example"), Some("explicit-key"), ("https://explicit.example", "explicit-key"))]
#[case::compactifai_absent_takes_environment(Host::Compactifai, &[("COMPACTIFAI_API_KEY", "env-key")], None, None, (COMPACTIFAI_DEFAULT, "env-key"))]
#[case::compactifai_empty_falls_through(Host::Compactifai, &[("COMPACTIFAI_API_KEY", "env-key")], Some(""), Some(""), (COMPACTIFAI_DEFAULT, "env-key"))]
#[case::compactifai_nothing_anywhere(Host::Compactifai, &[], None, None, (COMPACTIFAI_DEFAULT, ""))]
#[case::datarobot_nothing_set(Host::Datarobot, &[], None, None, (DATAROBOT_DEFAULT, "fake-api-key"))]
#[case::datarobot_environment_values(Host::Datarobot, &[("DATAROBOT_ENDPOINT", "https://tenant.example"), ("DATAROBOT_API_TOKEN", "env-token")], None, None, ("https://tenant.example", "env-token"))]
#[case::datarobot_explicit_values(Host::Datarobot, &[("DATAROBOT_ENDPOINT", "https://tenant.example"), ("DATAROBOT_API_TOKEN", "env-token")], Some("https://explicit.example"), Some("explicit-key"), ("https://explicit.example", "explicit-key"))]
#[case::datarobot_empty_explicit_takes_environment(Host::Datarobot, &[("DATAROBOT_ENDPOINT", "https://tenant.example"), ("DATAROBOT_API_TOKEN", "env-token")], Some(""), Some(""), ("https://tenant.example", "env-token"))]
#[case::datarobot_empty_endpoint_is_kept(Host::Datarobot, &[("DATAROBOT_ENDPOINT", "")], None, None, ("", "fake-api-key"))]
#[case::datarobot_empty_token_keeps_the_fake_key(Host::Datarobot, &[("DATAROBOT_API_TOKEN", "")], None, None, (DATAROBOT_DEFAULT, "fake-api-key"))]
fn resolution_follows_the_pinned_fallbacks(
    #[case] host: Host,
    #[case] environment: &[(&str, &str)],
    #[case] explicit_base: Option<&str>,
    #[case] explicit_key: Option<&str>,
    #[case] expected: (&str, &str),
) {
    let explicit = explicit_key.map(secret);
    let (resolved_base, resolved) = resolve(
        host,
        &settings(environment),
        explicit_base,
        explicit.as_ref(),
    );
    assert_eq!(resolved_base, expected.0);
    assert_eq!(resolved_key(&resolved), expected.1);
}
