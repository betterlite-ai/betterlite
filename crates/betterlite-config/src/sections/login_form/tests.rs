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
    assert!(!settings.hide_default_credentials_hint);
    assert_eq!(settings.hide_hint_environment, "false");
    assert!(settings.ui_password.expose_secret().is_empty());
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
    assert_eq!(settings.ui_password.expose_secret(), "s3cret");
}

#[test]
fn empty_password() {
    let settings = load("{}", &[("UI_PASSWORD", "")]).unwrap();
    assert!(settings.ui_password.expose_secret().is_empty());
}

#[test]
fn env_true_casefold() {
    // the hint predicate lowercases this text, so the loader keeps the spelling it was set with
    let settings = load("{}", &[("LITELLM_HIDE_DEFAULT_CREDENTIALS_HINT", "TRUE")]).unwrap();
    assert_eq!(settings.hide_hint_environment, "TRUE");
}

#[test]
fn env_whitespace_false() {
    // the surrounding spaces survive, so the predicate's exact "true" comparison does not match
    let settings = load("{}", &[("LITELLM_HIDE_DEFAULT_CREDENTIALS_HINT", " TRUE ")]).unwrap();
    assert_eq!(settings.hide_hint_environment, " TRUE ");
}

#[test]
fn env_numeric_false() {
    // "1" is not parsed as a bool here; only the predicate's text comparison decides
    let settings = load("{}", &[("LITELLM_HIDE_DEFAULT_CREDENTIALS_HINT", "1")]).unwrap();
    assert_eq!(settings.hide_hint_environment, "1");
}

#[rstest]
#[case::literal_true("general_settings: {hide_default_credentials_hint: true}", true)]
#[case::literal_false("general_settings: {hide_default_credentials_hint: false}", false)]
#[case::string_true("general_settings: {hide_default_credentials_hint: \"true\"}", false)]
#[case::null("general_settings: {hide_default_credentials_hint: null}", false)]
#[case::absent("{}", false)]
fn literal_boolean_only(#[case] source: &str, #[case] hidden: bool) {
    let settings = load(source, &[]).unwrap();
    assert_eq!(settings.hide_default_credentials_hint, hidden);
}

#[rstest]
#[case::setting_only(
    "general_settings: {hide_default_credentials_hint: true}",
    &[],
    true,
    "false",
    ""
)]
#[case::environment_only(
    "{}",
    &[("LITELLM_HIDE_DEFAULT_CREDENTIALS_HINT", "true")],
    false,
    "true",
    ""
)]
#[case::false_setting_with_true_environment(
    "general_settings: {hide_default_credentials_hint: false}",
    &[("LITELLM_HIDE_DEFAULT_CREDENTIALS_HINT", "true")],
    false,
    "true",
    ""
)]
#[case::password_with_false_setting(
    "general_settings: {hide_default_credentials_hint: false}",
    &[("UI_PASSWORD", "s3cret")],
    false,
    "false",
    "s3cret"
)]
fn hide_sources_or(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] setting: bool,
    #[case] hint_environment: &str,
    #[case] password: &str,
) {
    let settings = load(source, environment).unwrap();
    assert_eq!(settings.hide_default_credentials_hint, setting);
    assert_eq!(settings.hide_hint_environment, hint_environment);
    assert_eq!(settings.ui_password.expose_secret(), password);
}
