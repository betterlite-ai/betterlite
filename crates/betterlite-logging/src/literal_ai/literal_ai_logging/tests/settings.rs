#![cfg(test)]

use http::header::CONTENT_TYPE;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::settings::{
    API_KEY_HEADER, CLIENT_NAME, CLIENT_NAME_HEADER, DEFAULT_API_URL, ENVIRONMENT_HEADER,
    Overrides, Settings, SettingsError,
};
use super::support::variables;
use crate::batch::logger::DEFAULT_BATCH_SIZE;

fn resolved(pairs: &[(&str, &str)]) -> Settings {
    Settings::from_environment(&variables(pairs), Overrides::default()).unwrap()
}

#[test]
fn the_url_defaults_to_the_literal_cloud_and_the_environment_overrides_it() {
    assert_eq!(resolved(&[]).api_url, DEFAULT_API_URL);
    assert_eq!(
        resolved(&[("LITERAL_API_URL", "https://literal.internal")]).api_url,
        "https://literal.internal"
    );
}

#[test]
fn the_constructor_url_applies_only_when_the_environment_has_none() {
    let from_constructor = Settings::from_environment(
        &variables(&[]),
        Overrides {
            api_key: None,
            api_url: Some("https://constructor.local".to_owned()),
            environment: None,
        },
    )
    .unwrap();
    assert_eq!(from_constructor.api_url, "https://constructor.local");

    let from_variable = Settings::from_environment(
        &variables(&[("LITERAL_API_URL", "https://literal.internal")]),
        Overrides {
            api_key: None,
            api_url: Some("https://constructor.local".to_owned()),
            environment: None,
        },
    )
    .unwrap();
    assert_eq!(from_variable.api_url, "https://literal.internal");
}

#[test]
fn the_constructor_key_wins_over_the_environment_key() {
    let configured = Settings::from_environment(
        &variables(&[("LITERAL_API_KEY", "environment-key")]),
        Overrides {
            api_key: Some(SecretString::from("constructor-key")),
            api_url: None,
            environment: None,
        },
    )
    .unwrap();
    assert_eq!(
        configured.api_key.as_ref().unwrap().expose_secret(),
        "constructor-key"
    );
}

#[rstest]
#[case::no_key(&[], None)]
#[case::environment_key(&[("LITERAL_API_KEY", "environment-key")], Some("environment-key"))]
#[case::empty_environment_key(&[("LITERAL_API_KEY", "")], None)]
fn the_api_key_comes_from_the_environment(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let configured = resolved(pairs);
    let key = configured
        .api_key
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned());
    assert_eq!(key.as_deref(), expected);
}

#[test]
fn an_empty_url_counts_as_unset() {
    assert_eq!(
        resolved(&[("LITERAL_API_URL", "")]).api_url,
        DEFAULT_API_URL
    );
}

#[test]
fn the_batch_size_defaults_to_the_batch_logger_default() {
    assert_eq!(resolved(&[]).batch_size, DEFAULT_BATCH_SIZE);
    assert_eq!(resolved(&[("LITERAL_BATCH_SIZE", "")]).batch_size, 512);
}

#[rstest]
#[case::plain("4", 4)]
#[case::surrounded_by_spaces(" 4 ", 4)]
#[case::zero("0", 0)]
#[case::negative("-2", 0)]
fn the_batch_size_comes_from_the_environment(#[case] text: &str, #[case] expected: usize) {
    assert_eq!(
        resolved(&[("LITERAL_BATCH_SIZE", text)]).batch_size,
        expected
    );
}

#[test]
fn an_invalid_batch_size_is_an_error_that_names_the_variable() {
    let error = Settings::from_environment(
        &variables(&[("LITERAL_BATCH_SIZE", "two")]),
        Overrides::default(),
    )
    .unwrap_err();
    assert!(matches!(error, SettingsError::Environment(_)));
    assert!(error.to_string().contains("LITERAL_BATCH_SIZE"));
}

#[test]
fn the_headers_carry_the_client_name_the_content_type_and_the_api_key() {
    let configured = resolved(&[("LITERAL_API_KEY", "literal-key")]);
    let headers = configured.headers().unwrap();
    assert_eq!(headers[CONTENT_TYPE], "application/json");
    assert_eq!(headers[CLIENT_NAME_HEADER], CLIENT_NAME);
    assert_eq!(headers[API_KEY_HEADER], "literal-key");
    assert!(headers[API_KEY_HEADER].is_sensitive());
    assert!(!headers.contains_key(ENVIRONMENT_HEADER));
}

#[test]
fn the_constructor_environment_adds_the_environment_header() {
    let configured = Settings::from_environment(
        &variables(&[("LITERAL_API_KEY", "literal-key")]),
        Overrides {
            api_key: None,
            api_url: None,
            environment: Some("production".to_owned()),
        },
    )
    .unwrap();
    assert_eq!(
        configured.headers().unwrap()[ENVIRONMENT_HEADER],
        "production"
    );
}
