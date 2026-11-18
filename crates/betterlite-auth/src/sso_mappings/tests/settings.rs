use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::sso_mappings::settings::{DEFAULT_GENERIC_SCOPE, MissingVariable, generic_sso_settings};

use super::support::sources;

fn required() -> [(&'static str, &'static str); 4] {
    [
        ("GENERIC_CLIENT_SECRET", "secret"),
        (
            "GENERIC_AUTHORIZATION_ENDPOINT",
            "https://idp.example/authorize",
        ),
        ("GENERIC_TOKEN_ENDPOINT", "https://idp.example/token"),
        ("GENERIC_USERINFO_ENDPOINT", "https://idp.example/userinfo"),
    ]
}

#[test]
fn resolves_the_generic_endpoint_settings() {
    let settings = generic_sso_settings(&sources(&required())).expect("the settings");
    assert_eq!(settings.client_secret.expose_secret(), "secret");
    assert_eq!(DEFAULT_GENERIC_SCOPE, "openid email profile");
    assert_eq!(
        settings.scope,
        vec![
            "openid".to_owned(),
            "email".to_owned(),
            "profile".to_owned()
        ]
    );
    assert_eq!(
        settings.authorization_endpoint,
        "https://idp.example/authorize"
    );
    assert_eq!(settings.token_endpoint, "https://idp.example/token");
    assert_eq!(settings.userinfo_endpoint, "https://idp.example/userinfo");
    assert!(!settings.include_client_id);
}

#[test]
fn an_empty_secret_is_accepted() {
    let mut pairs = required();
    pairs[0].1 = "";
    let settings = generic_sso_settings(&sources(&pairs)).expect("the settings");
    assert_eq!(settings.client_secret.expose_secret(), "");
}

#[rstest]
#[case::empty("", vec![String::new()])]
#[case::single("openid", vec!["openid".to_owned()])]
#[case::double_space(
    "openid  email",
    vec!["openid".to_owned(), String::new(), "email".to_owned()]
)]
#[case::tab_is_not_a_separator("openid\temail", vec!["openid\temail".to_owned()])]
fn splits_the_scope_on_literal_spaces(#[case] scope: &'static str, #[case] expected: Vec<String>) {
    let mut pairs = required().to_vec();
    pairs.push(("GENERIC_SCOPE", scope));
    let settings = generic_sso_settings(&sources(&pairs)).expect("the settings");
    assert_eq!(settings.scope, expected);
}

#[test]
fn the_include_client_id_flag_is_passed_through() {
    let mut pairs = required().to_vec();
    pairs.push(("GENERIC_INCLUDE_CLIENT_ID", "true"));
    let settings = generic_sso_settings(&sources(&pairs)).expect("the settings");
    assert!(settings.include_client_id);
}

#[rstest]
#[case::secret(0, "GENERIC_CLIENT_SECRET")]
#[case::authorization_endpoint(1, "GENERIC_AUTHORIZATION_ENDPOINT")]
#[case::token_endpoint(2, "GENERIC_TOKEN_ENDPOINT")]
#[case::userinfo_endpoint(3, "GENERIC_USERINFO_ENDPOINT")]
fn reports_the_first_missing_variable_in_source_order(
    #[case] present: usize,
    #[case] variable: &str,
) {
    let pairs: Vec<(&str, &str)> = required().into_iter().take(present).collect();
    let error = generic_sso_settings(&sources(&pairs)).expect_err("the missing variable");
    let MissingVariable(exception) = error;
    assert_eq!(
        exception.message,
        format!("{variable} not set. Set it in .env file")
    );
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some(variable));
    assert_eq!(exception.code, "500");
}
