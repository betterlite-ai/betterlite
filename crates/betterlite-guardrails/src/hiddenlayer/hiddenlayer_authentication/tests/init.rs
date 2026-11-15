use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_http::proxy::base_builder;
use rstest::rstest;
use secrecy::SecretString;

use super::super::error::HiddenlayerError;
use super::super::settings::{
    DEFAULT_API_BASE, DEFAULT_AUTH_TIMEOUT, DEFAULT_AUTH_URL, HiddenlayerCredentials, resolve,
};
use super::super::token::{JwtError, JwtTransport};
use super::super::{HiddenlayerAuth, is_saas};
use super::support::{self, TokenScript};

fn credentials(
    api_base: Option<&str>,
    api_id: Option<&str>,
    api_key: Option<&str>,
) -> HiddenlayerCredentials {
    HiddenlayerCredentials {
        api_id: api_id.map(str::to_owned),
        api_key: api_key.map(SecretString::from),
        api_base: api_base.map(str::to_owned),
        auth_url: None,
        timeout: None,
    }
}

fn build(
    input: &HiddenlayerCredentials,
    env: &BTreeMap<String, String>,
    script: &Arc<TokenScript>,
) -> Result<HiddenlayerAuth, HiddenlayerError> {
    let client = base_builder()
        .build()
        .map_err(|source| HiddenlayerError::Transport { source })?;
    let transport: Arc<dyn JwtTransport> = Arc::<TokenScript>::clone(script);
    HiddenlayerAuth::new(input, env, client, transport)
}

fn failure(
    input: &HiddenlayerCredentials,
    env: &BTreeMap<String, String>,
    script: &Arc<TokenScript>,
) -> HiddenlayerError {
    match build(input, env, script) {
        Ok(_auth) => panic!("the Hiddenlayer guardrail must not be constructed"),
        Err(error) => error,
    }
}

#[rstest]
#[case("https://api.hiddenlayer.ai", true)]
#[case("https://api.hiddenlayer.ai/v1", true)]
#[case("http://api.hiddenlayer.ai:8443", true)]
#[case("https://tenant.hiddenlayer.ai", true)]
#[case("https://not-hiddenlayer.ai.evil.test", false)]
#[case("https://hiddenlayer.ai.evil.test", false)]
#[case("https://api.hiddenlayer.ai.evil.test", false)]
#[case("api.hiddenlayer.ai", false)]
#[case("http://127.0.0.1:8080", false)]
#[case("", false)]
fn is_saas_detects_the_hiddenlayer_hosts(#[case] api_base: &str, #[case] expected: bool) {
    assert_eq!(is_saas(api_base), expected);
}

#[test]
fn saas_requires_an_api_id() {
    let script = support::tokens(&[]);
    let input = credentials(Some("https://api.hiddenlayer.ai"), None, Some("secret"));
    let error = failure(&input, &BTreeMap::new(), &script);
    assert!(matches!(error, HiddenlayerError::MissingApiId));
    assert_eq!(
        error.to_string(),
        "`api_id` cannot be None when using the SaaS version of HiddenLayer."
    );
    assert!(script.requests().is_empty());
}

#[test]
fn saas_requires_an_api_key() {
    let script = support::tokens(&[]);
    let input = credentials(Some("https://api.hiddenlayer.ai"), Some("client"), None);
    let error = failure(&input, &BTreeMap::new(), &script);
    assert!(matches!(error, HiddenlayerError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "`api_key` cannot be None when using the SaaS version of HiddenLayer."
    );
    assert!(script.requests().is_empty());
}

#[test]
fn on_premises_skips_the_token_request() {
    let script = support::tokens(&["unused"]);
    let input = credentials(Some("http://127.0.0.1:9"), Some("client"), Some("secret"));
    let auth = build(&input, &BTreeMap::new(), &script).unwrap();
    assert!(!support::has_token(&auth));
    assert!(script.requests().is_empty());
}

#[test]
fn on_premises_needs_no_credentials() {
    let script = support::tokens(&[]);
    let input = credentials(Some("http://127.0.0.1:9"), None, None);
    let auth = build(&input, &BTreeMap::new(), &script).unwrap();
    assert!(!support::has_token(&auth));
}

#[test]
fn saas_fetches_the_token_at_construction() {
    let script = support::tokens(&["token-1"]);
    let input = credentials(
        Some("https://api.hiddenlayer.ai"),
        Some("client"),
        Some("secret"),
    );
    let auth = build(&input, &BTreeMap::new(), &script).unwrap();
    assert!(support::has_token(&auth));
    let calls = script.requests();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].url,
        "https://auth.hiddenlayer.ai/oauth2/token?grant_type=client_credentials"
    );
    assert_eq!(calls[0].api_id, "client");
    assert_eq!(calls[0].api_key, "secret");
    assert_eq!(calls[0].timeout, DEFAULT_AUTH_TIMEOUT);
}

#[test]
fn the_environment_supplies_missing_values() {
    let script = support::tokens(&["token-1"]);
    let env = BTreeMap::from([
        (
            "HIDDENLAYER_API_BASE".to_owned(),
            "https://api.hiddenlayer.ai".to_owned(),
        ),
        (
            "HIDDENLAYER_AUTH_URL".to_owned(),
            "https://auth.example.test".to_owned(),
        ),
        ("HIDDENLAYER_CLIENT_ID".to_owned(), "env-client".to_owned()),
        (
            "HIDDENLAYER_CLIENT_SECRET".to_owned(),
            "env-secret".to_owned(),
        ),
    ]);
    let auth = build(&HiddenlayerCredentials::default(), &env, &script).unwrap();
    assert!(support::has_token(&auth));
    let calls = script.requests();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].url,
        "https://auth.example.test/oauth2/token?grant_type=client_credentials"
    );
    assert_eq!(calls[0].api_id, "env-client");
    assert_eq!(calls[0].api_key, "env-secret");
}

#[test]
fn constructor_credentials_win_over_the_environment() {
    let script = support::tokens(&["token-1"]);
    let env = BTreeMap::from([
        (
            "HIDDENLAYER_API_BASE".to_owned(),
            "https://api.hiddenlayer.ai".to_owned(),
        ),
        (
            "HIDDENLAYER_AUTH_URL".to_owned(),
            "https://auth.example.test".to_owned(),
        ),
        ("HIDDENLAYER_CLIENT_ID".to_owned(), "env-client".to_owned()),
        (
            "HIDDENLAYER_CLIENT_SECRET".to_owned(),
            "env-secret".to_owned(),
        ),
    ]);
    let mut input = credentials(
        Some("https://api.hiddenlayer.ai"),
        Some("explicit-client"),
        Some("explicit-secret"),
    );
    input.auth_url = Some("https://auth.explicit.test".to_owned());
    build(&input, &env, &script).unwrap();
    let calls = script.requests();
    assert_eq!(
        calls[0].url,
        "https://auth.explicit.test/oauth2/token?grant_type=client_credentials"
    );
    assert_eq!(calls[0].api_id, "explicit-client");
    assert_eq!(calls[0].api_key, "explicit-secret");
}

#[test]
fn an_empty_environment_value_is_unset() {
    let script = support::tokens(&[]);
    let env = BTreeMap::from([
        (
            "HIDDENLAYER_API_BASE".to_owned(),
            "https://api.hiddenlayer.ai".to_owned(),
        ),
        ("HIDDENLAYER_CLIENT_ID".to_owned(), String::new()),
        ("HIDDENLAYER_CLIENT_SECRET".to_owned(), String::new()),
    ]);
    let error = failure(&HiddenlayerCredentials::default(), &env, &script);
    assert!(matches!(error, HiddenlayerError::MissingApiId));
}

#[test]
fn saas_requires_credentials_at_resolution() {
    let missing_id = resolve(
        &credentials(Some("https://api.hiddenlayer.ai"), None, Some("secret")),
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert!(matches!(missing_id, HiddenlayerError::MissingApiId));
    let missing_key = resolve(
        &credentials(Some("https://api.hiddenlayer.ai"), Some("client"), None),
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert!(matches!(missing_key, HiddenlayerError::MissingApiKey));
}

#[test]
fn an_empty_environment_secret_is_unset() {
    let env = BTreeMap::from([
        (
            "HIDDENLAYER_API_BASE".to_owned(),
            "http://127.0.0.1:9".to_owned(),
        ),
        ("HIDDENLAYER_CLIENT_SECRET".to_owned(), String::new()),
    ]);
    let settings = resolve(&HiddenlayerCredentials::default(), &env).unwrap();
    assert!(settings.api_key.is_none());
}

#[test]
fn an_empty_environment_auth_url_is_unset() {
    let script = support::tokens(&["token-1"]);
    let env = BTreeMap::from([
        (
            "HIDDENLAYER_API_BASE".to_owned(),
            "https://api.hiddenlayer.ai".to_owned(),
        ),
        ("HIDDENLAYER_AUTH_URL".to_owned(), String::new()),
        ("HIDDENLAYER_CLIENT_ID".to_owned(), "client".to_owned()),
        ("HIDDENLAYER_CLIENT_SECRET".to_owned(), "secret".to_owned()),
    ]);
    let auth = build(&HiddenlayerCredentials::default(), &env, &script).unwrap();
    assert!(support::has_token(&auth));
    assert_eq!(
        script.requests()[0].url,
        format!("{DEFAULT_AUTH_URL}/oauth2/token?grant_type=client_credentials")
    );
}

#[test]
fn the_default_base_is_the_saas_platform() {
    assert_eq!(DEFAULT_API_BASE, "https://api.hiddenlayer.ai");
    assert!(is_saas(DEFAULT_API_BASE));
}

#[test]
fn the_default_auth_url_is_used_for_the_token_request() {
    let script = support::tokens(&["token-1"]);
    let input = credentials(None, Some("client"), Some("secret"));
    build(&input, &BTreeMap::new(), &script).unwrap();
    assert_eq!(
        script.requests()[0].url,
        format!("{DEFAULT_AUTH_URL}/oauth2/token?grant_type=client_credentials")
    );
}

#[test]
fn the_custom_timeout_reaches_the_token_request() {
    let script = support::tokens(&["token-1"]);
    let mut input = credentials(
        Some("https://api.hiddenlayer.ai"),
        Some("client"),
        Some("secret"),
    );
    input.timeout = Some(2.5_f64);
    build(&input, &BTreeMap::new(), &script).unwrap();
    assert_eq!(script.requests()[0].timeout, Duration::from_millis(2500));
}

#[test]
fn the_default_timeout_is_thirty_seconds() {
    assert_eq!(DEFAULT_AUTH_TIMEOUT, Duration::from_secs(30));
}

#[rstest]
#[case::negative(-1.0_f64)]
#[case::not_a_number(f64::NAN)]
#[case::infinite(f64::INFINITY)]
fn an_unusable_timeout_is_refused(#[case] timeout: f64) {
    let script = support::tokens(&["token-1"]);
    let mut input = credentials(
        Some("https://api.hiddenlayer.ai"),
        Some("client"),
        Some("secret"),
    );
    input.timeout = Some(timeout);
    let error = failure(&input, &BTreeMap::new(), &script);
    assert!(matches!(error, HiddenlayerError::InvalidTimeout { .. }));
    assert!(script.requests().is_empty());
}

#[test]
fn a_failed_token_fetch_fails_the_constructor() {
    let script = support::failing(JwtError::Status {
        status: 401,
        body: "denied".to_owned(),
    });
    let input = credentials(
        Some("https://api.hiddenlayer.ai"),
        Some("client"),
        Some("secret"),
    );
    let error = failure(&input, &BTreeMap::new(), &script);
    assert!(matches!(
        error,
        HiddenlayerError::Jwt(JwtError::Status { status: 401, .. })
    ));
    assert_eq!(
        error.to_string(),
        "Unable to get authentication credentials for the HiddenLayer API: 401: denied"
    );
}
