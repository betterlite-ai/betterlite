#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::login_form::Settings;

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load(source)?.sections.login_form)
}

#[test]
fn empty_url_defaults() {
    let settings = load("{}", &[]).unwrap();
    assert_eq!(settings.proxy_base_url, "");
    assert_eq!(settings.server_root_path, "");
}

#[test]
fn raw_url_concatenation() {
    // the renderer concatenates these two values literally, so the trailing slash and the root stay as configured
    let settings = load(
        "{}",
        &[
            ("PROXY_BASE_URL", "https://proxy.example.com/base/"),
            ("SERVER_ROOT_PATH", "/gateway"),
        ],
    )
    .unwrap();
    assert_eq!(settings.proxy_base_url, "https://proxy.example.com/base/");
    assert_eq!(settings.server_root_path, "/gateway");
}

#[test]
fn nonempty_password() {
    let settings = load("{}", &[("UI_PASSWORD", "s3cret")]).unwrap();
    let password = settings.ui_password.unwrap();
    assert!(!password.expose_secret().is_empty());
}

#[test]
fn empty_password() {
    let settings = load("{}", &[("UI_PASSWORD", "")]).unwrap();
    let password = settings.ui_password.unwrap();
    assert!(password.expose_secret().is_empty());
}

#[test]
fn env_true_casefold() {
    // the hint predicate lowercases this text, so the loader keeps the spelling it was set with
    let settings = load("{}", &[("LITELLM_HIDE_DEFAULT_CREDENTIALS_HINT", "TRUE")]).unwrap();
    assert_eq!(settings.hide_hint_environment.as_deref(), Some("TRUE"));
}

#[test]
fn env_whitespace_false() {
    // the surrounding spaces survive, so the predicate's exact "true" comparison does not match
    let settings = load("{}", &[("LITELLM_HIDE_DEFAULT_CREDENTIALS_HINT", " TRUE ")]).unwrap();
    assert_eq!(settings.hide_hint_environment.as_deref(), Some(" TRUE "));
}

#[test]
fn env_numeric_false() {
    // "1" is not parsed as a bool here; only the predicate's text comparison decides
    let settings = load("{}", &[("LITELLM_HIDE_DEFAULT_CREDENTIALS_HINT", "1")]).unwrap();
    assert_eq!(settings.hide_hint_environment.as_deref(), Some("1"));
}

#[rstest]
#[case::literal_true("general_settings: {hide_default_credentials_hint: true}", Some(true))]
#[case::literal_false(
    "general_settings: {hide_default_credentials_hint: false}",
    Some(false)
)]
#[case::string_true(
    "general_settings: {hide_default_credentials_hint: \"true\"}",
    Some(false)
)]
#[case::null("general_settings: {hide_default_credentials_hint: null}", None)]
#[case::absent("{}", None)]
fn literal_boolean_only(#[case] source: &str, #[case] loaded: Option<bool>) {
    let settings = load(source, &[]).unwrap();
    assert_eq!(settings.hide_default_credentials_hint, loaded);
}

#[rstest]
#[case::setting_only("general_settings: {hide_default_credentials_hint: true}", &[], Some(true))]
#[case::environment_only(
    "{}",
    &[("LITELLM_HIDE_DEFAULT_CREDENTIALS_HINT", "true")],
    None
)]
#[case::password_only("{}", &[("UI_PASSWORD", "s3cret")], None)]
#[case::none("general_settings: {hide_default_credentials_hint: false}", &[], Some(false))]
fn hide_sources_or(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] setting: Option<bool>,
) {
    let settings = load(source, environment).unwrap();
    assert_eq!(settings.hide_default_credentials_hint, setting);
    assert_eq!(
        settings.hide_hint_environment.is_some(),
        environment
            .iter()
            .any(|(name, _)| *name == "LITELLM_HIDE_DEFAULT_CREDENTIALS_HINT")
    );
    assert_eq!(
        settings.ui_password.is_some(),
        environment.iter().any(|(name, _)| *name == "UI_PASSWORD")
    );
}
