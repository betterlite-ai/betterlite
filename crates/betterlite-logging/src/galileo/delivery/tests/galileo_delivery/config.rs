use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::super::super::config::{CLOUD_API_BASE_URL, GalileoConfig};
use super::{API_KEY_ENV, TOKEN_ENV, delivery};

fn config_from(variables: &[(&str, &str)]) -> GalileoConfig {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    GalileoConfig::from_environment(&environment)
}

#[test]
fn reads_every_galileo_variable_from_the_environment() {
    let config = config_from(&[
        ("GALILEO_API_KEY", "key"),
        ("GALILEO_PROJECT_ID", "project"),
        ("GALILEO_LOG_STREAM_ID", "stream"),
        ("GALILEO_USERNAME", "user"),
        ("GALILEO_PASSWORD", "pass"),
        ("GALILEO_BASE_URL", "https://galileo.example"),
    ]);
    assert_eq!(config.api_key.unwrap().expose_secret(), "key");
    assert_eq!(config.project_id.as_deref(), Some("project"));
    assert_eq!(config.log_stream_id.as_deref(), Some("stream"));
    assert_eq!(config.username.as_deref(), Some("user"));
    assert_eq!(config.password.unwrap().expose_secret(), "pass");
    assert_eq!(config.base_url.as_deref(), Some("https://galileo.example"));
}

#[test]
fn batch_size_defaults_to_one() {
    assert_eq!(config_from(&[]).batch_size, 1);
}

#[rstest]
#[case::one("https://galileo.example/", "https://galileo.example")]
#[case::many("https://galileo.example///", "https://galileo.example")]
#[case::none("https://galileo.example", "https://galileo.example")]
fn base_url_loses_trailing_slashes(#[case] given: &str, #[case] expected: &str) {
    let config = config_from(&[("GALILEO_BASE_URL", given)]);
    assert_eq!(config.base_url.as_deref(), Some(expected));
}

#[test]
fn api_key_without_a_base_url_uses_the_galileo_cloud() {
    let config = delivery(API_KEY_ENV).config;
    assert_eq!(config.base_url.as_deref(), Some(CLOUD_API_BASE_URL));
    assert_eq!(CLOUD_API_BASE_URL, "https://api.galileo.ai");
    assert!(config.use_v2_api);
}

#[test]
fn username_and_password_get_no_default_base_url() {
    let config = config_from(&[("GALILEO_USERNAME", "user"), ("GALILEO_PASSWORD", "pass")]);
    assert_eq!(config.base_url, None);
    assert!(!config.use_v2_api);
}

#[test]
fn a_base_url_of_only_slashes_counts_as_unset() {
    let config = config_from(&[("GALILEO_API_KEY", "key"), ("GALILEO_BASE_URL", "//")]);
    assert_eq!(config.base_url.as_deref(), Some(CLOUD_API_BASE_URL));
}

#[test]
fn an_empty_variable_counts_as_unset() {
    let config = config_from(&[
        ("GALILEO_API_KEY", ""),
        ("GALILEO_PROJECT_ID", ""),
        ("GALILEO_USERNAME", ""),
    ]);
    assert!(config.api_key.is_none());
    assert!(!config.use_v2_api);
    assert_eq!(config.project_id, None);
    assert_eq!(config.username, None);
}

#[rstest]
#[case::api_key(API_KEY_ENV, true)]
#[case::token_auth(TOKEN_ENV, true)]
#[case::nothing(&[], false)]
#[case::api_key_without_project(&[("GALILEO_API_KEY", "key")], false)]
#[case::token_auth_without_base_url(
    &[("GALILEO_USERNAME", "u"), ("GALILEO_PASSWORD", "p"), ("GALILEO_PROJECT_ID", "x")],
    false
)]
#[case::token_auth_without_password(
    &[("GALILEO_USERNAME", "u"), ("GALILEO_PROJECT_ID", "x"), ("GALILEO_BASE_URL", "https://g")],
    false
)]
#[case::token_auth_without_username(
    &[("GALILEO_PASSWORD", "p"), ("GALILEO_PROJECT_ID", "x"), ("GALILEO_BASE_URL", "https://g")],
    false
)]
fn is_configured_needs_project_base_url_and_credentials(
    #[case] variables: &[(&str, &str)],
    #[case] expected: bool,
) {
    assert_eq!(config_from(variables).is_configured(), expected);
}
