#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::gemini_live_setup::endpoint::{DEFAULT_STUDIO_LIVE_BASE, connect_url};
use crate::gemini_live_setup::error::LiveSetupError;

const RPC_PATH: &str =
    "/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

fn key(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

#[test]
fn the_default_base_is_the_studio_live_endpoint() {
    assert_eq!(
        DEFAULT_STUDIO_LIVE_BASE,
        "wss://generativelanguage.googleapis.com"
    );
    let url = connect_url(None, Some(&key("studio-key")), None).expect("the url builds");
    assert_eq!(
        url.expose_secret(),
        format!("{DEFAULT_STUDIO_LIVE_BASE}{RPC_PATH}?key=studio-key")
    );
}

#[rstest]
#[case::https("https://example.test", "wss://example.test")]
#[case::http("http://localhost:8080", "ws://localhost:8080")]
#[case::secure_websocket("wss://example.test", "wss://example.test")]
#[case::plain_websocket("ws://example.test", "ws://example.test")]
fn the_base_scheme_is_converted_for_the_websocket(#[case] base: &str, #[case] converted: &str) {
    let url = connect_url(Some(base), Some(&key("studio-key")), None).expect("the url builds");
    assert_eq!(
        url.expose_secret(),
        format!("{converted}{RPC_PATH}?key=studio-key")
    );
}

#[test]
fn an_explicit_empty_key_is_kept() {
    let url =
        connect_url(None, Some(&key("")), Some(&key("environment-key"))).expect("the url builds");
    assert_eq!(
        url.expose_secret(),
        format!("{DEFAULT_STUDIO_LIVE_BASE}{RPC_PATH}?key=")
    );
}

#[test]
fn an_explicit_key_wins_over_the_environment_key() {
    let url = connect_url(
        None,
        Some(&key("explicit-key")),
        Some(&key("environment-key")),
    )
    .expect("the url builds");
    assert!(url.expose_secret().ends_with("?key=explicit-key"));
}

#[test]
fn the_environment_key_is_used_without_an_explicit_key() {
    let url = connect_url(None, None, Some(&key("environment-key"))).expect("the url builds");
    assert!(url.expose_secret().ends_with("?key=environment-key"));
}

#[test]
fn a_missing_key_is_refused() {
    let error = connect_url(None, None, None).expect_err("a key is required");
    assert!(matches!(&error, LiveSetupError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "api_key is required for Gemini API calls"
    );
}

#[test]
fn the_connect_url_redacts_the_key_from_diagnostics() {
    let url = connect_url(None, Some(&key("secret-key")), None).expect("the url builds");
    assert!(!format!("{url:?}").contains("secret-key"));
}
