#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::compatible_embedding_settings::{
    DEFAULT_COMETAPI_API_BASE, Globals, Settings,
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
        .compatible_embedding_settings
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn exposed(value: Option<&SecretString>) -> Option<String> {
    value.map(|secret| secret.expose_secret().to_owned())
}

const ENVIRONMENT: &[(&str, &str)] = &[
    ("OPENAI_LIKE_API_BASE", "http://openai-like/v1"),
    ("OPENAI_LIKE_API_KEY", "sk-openai-like"),
    ("COMETAPI_API_BASE", "https://comet.example/v1"),
    ("COMETAPI_KEY", "sk-cometapi"),
];

#[rstest]
#[case::all_four(
    ENVIRONMENT,
    Some("http://openai-like/v1"),
    Some("sk-openai-like"),
    Some("https://comet.example/v1"),
    Some("sk-cometapi")
)]
#[case::openai_like_only(&[("OPENAI_LIKE_API_BASE", "http://openai-like/v1"), ("OPENAI_LIKE_API_KEY", "sk-openai-like")], Some("http://openai-like/v1"), Some("sk-openai-like"), None, None)]
#[case::cometapi_only(&[("COMETAPI_API_BASE", "https://comet.example/v1"), ("COMETAPI_KEY", "sk-cometapi")], None, None, Some("https://comet.example/v1"), Some("sk-cometapi"))]
#[case::empty_values_count_as_unset(&[("OPENAI_LIKE_API_BASE", ""), ("OPENAI_LIKE_API_KEY", ""), ("COMETAPI_API_BASE", ""), ("COMETAPI_KEY", "")], None, None, None, None)]
#[case::absent(&[], None, None, None, None)]
fn loads_every_environment_alias(
    #[case] environment: &[(&str, &str)],
    #[case] openai_like_base: Option<&str>,
    #[case] openai_like_key: Option<&str>,
    #[case] cometapi_base: Option<&str>,
    #[case] cometapi_key: Option<&str>,
) {
    let loaded = settings(environment);
    assert_eq!(loaded.openai_like.api_base.as_deref(), openai_like_base);
    assert_eq!(
        exposed(loaded.openai_like.api_key.as_ref()).as_deref(),
        openai_like_key
    );
    assert_eq!(loaded.cometapi_api_base.as_deref(), cometapi_base);
    assert_eq!(
        exposed(loaded.cometapi_credentials.api_key.as_ref()).as_deref(),
        cometapi_key
    );
}

#[rstest]
#[case::explicit_wins(
    ENVIRONMENT,
    globals(None, None, None, None),
    Some("http://explicit/v1"),
    Some("sk-explicit"),
    Some("http://explicit/v1"),
    Some("sk-explicit")
)]
#[case::global_base_and_key_win(
    ENVIRONMENT,
    globals(Some("https://global/v1"), Some("sk-global"), None, None),
    None,
    None,
    Some("https://global/v1"),
    Some("sk-global")
)]
#[case::openai_like_global_key(
    ENVIRONMENT,
    globals(None, None, Some("sk-like"), None),
    None,
    None,
    Some("http://openai-like/v1"),
    Some("sk-like")
)]
#[case::environment_takes_over(
    ENVIRONMENT,
    globals(None, None, None, None),
    None,
    None,
    Some("http://openai-like/v1"),
    Some("sk-openai-like")
)]
#[case::nothing_anywhere(&[], globals(None, None, None, None), None, None, None, None)]
#[case::explicit_empty_key_suppresses(
    ENVIRONMENT,
    globals(None, Some("sk-global"), None, None),
    None,
    Some(""),
    Some("http://openai-like/v1"),
    Some("")
)]
#[case::explicit_empty_base_is_used_as_given(&[], globals(None, None, None, None), Some(""), None, Some(""), None)]
fn openai_like_identity_follows_the_pinned_precedence(
    #[case] environment: &[(&str, &str)],
    #[case] globals: Globals,
    #[case] explicit_base: Option<&str>,
    #[case] explicit_key: Option<&str>,
    #[case] expected_base: Option<&str>,
    #[case] expected_key: Option<&str>,
) {
    let supplied_key = explicit_key.map(secret);
    let (base, key) =
        settings(environment).openai_like_identity(&globals, explicit_base, supplied_key.as_ref());
    assert_eq!(base.as_deref(), expected_base);
    assert_eq!(exposed(key.as_ref()).as_deref(), expected_key);
}

#[rstest]
#[case::explicit_wins(
    ENVIRONMENT,
    globals(None, None, None, None),
    Some("https://explicit/v1"),
    Some("sk-explicit"),
    "https://explicit/v1",
    Some("sk-explicit")
)]
#[case::global_base_and_cometapi_key(
    ENVIRONMENT,
    globals(
        Some("https://global/v1"),
        Some("sk-api"),
        Some("sk-like"),
        Some("sk-cometapi-global")
    ),
    None,
    None,
    "https://global/v1",
    Some("sk-cometapi-global")
)]
#[case::cometapi_environment(
    ENVIRONMENT,
    globals(None, None, None, None),
    None,
    None,
    "https://comet.example/v1",
    Some("sk-cometapi")
)]
#[case::general_global_key_comes_last(
    ENVIRONMENT,
    globals(None, Some("sk-api"), None, None),
    None,
    None,
    "https://comet.example/v1",
    Some("sk-cometapi")
)]
#[case::general_global_key_without_environment(&[], globals(None, Some("sk-api"), None, None), None, None, DEFAULT_COMETAPI_API_BASE, Some("sk-api"))]
#[case::default_base_and_no_key(&[], globals(None, None, None, None), None, None, DEFAULT_COMETAPI_API_BASE, None)]
#[case::explicit_empty_key_suppresses(
    ENVIRONMENT,
    globals(None, Some("sk-api"), None, Some("sk-cometapi-global")),
    None,
    Some(""),
    "https://comet.example/v1",
    Some("")
)]
#[case::explicit_empty_base_is_used_as_given(&[], globals(None, None, None, None), Some(""), None, "", None)]
fn cometapi_identity_follows_the_pinned_precedence(
    #[case] environment: &[(&str, &str)],
    #[case] globals: Globals,
    #[case] explicit_base: Option<&str>,
    #[case] explicit_key: Option<&str>,
    #[case] expected_base: &str,
    #[case] expected_key: Option<&str>,
) {
    let supplied_key = explicit_key.map(secret);
    let (base, key) =
        settings(environment).cometapi_identity(&globals, explicit_base, supplied_key.as_ref());
    assert_eq!(base, expected_base);
    assert_eq!(exposed(key.as_ref()).as_deref(), expected_key);
}

fn globals(
    api_base: Option<&str>,
    api_key: Option<&str>,
    openai_like_key: Option<&str>,
    cometapi_key: Option<&str>,
) -> Globals {
    Globals {
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(secret),
        openai_like_key: openai_like_key.map(secret),
        cometapi_key: cometapi_key.map(secret),
    }
}
