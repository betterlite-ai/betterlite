#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::ready;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use serde::de::IgnoredAny;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::models::{
    MODEL_LIST_LIMIT, ModelsTransport, decode_anthropic_models, get_models, models_request,
};
use super::{
    ANTHROPIC_VERSION, AnthropicSettings, BROWSER_ACCESS_HEADER, CredentialsError,
    DEFAULT_API_BASE, OAUTH_BETA_HEADER, OAUTH_BETA_HEADER_NAME, OauthTokenClaim, get_auth_header,
    get_base_model, handle_anthropic_oauth, is_anthropic_oauth_key, make_api_key_auth_header,
    resolve_oauth_token_claim,
};

const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct Sent {
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    sensitive: Vec<String>,
}

struct Script {
    reply: Option<(u16, Vec<u8>)>,
    sent: Mutex<Vec<Sent>>,
}

impl Script {
    fn replying(status: u16, body: &[u8]) -> Self {
        Self {
            reply: Some((status, body.to_vec())),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            reply: None,
            sent: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }
}

impl ModelsTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let mut headers: Vec<(String, String)> = request
            .headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_str().unwrap().to_owned()))
            .collect();
        headers.sort();
        let mut sensitive: Vec<String> = request
            .headers
            .iter()
            .filter(|(_, value)| value.is_sensitive())
            .map(|(name, _)| name.to_string())
            .collect();
        sensitive.sort();
        self.sent.lock().unwrap().push(Sent {
            method: request.method,
            url: request.url.to_string(),
            headers,
            sensitive,
        });
        ready(
            self.reply
                .clone()
                .ok_or(TransportError::Closed)
                .map(|(status, body)| UpstreamResponse {
                    status,
                    headers: HeaderMap::new(),
                    body,
                }),
        )
    }
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn revealed(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

fn configured(
    api_key: Option<&str>,
    auth_token: Option<&str>,
    api_base: Option<&str>,
) -> AnthropicSettings {
    AnthropicSettings {
        api_key: api_key.map(secret),
        auth_token: auth_token.map(secret),
        api_base: api_base.map_or_else(|| DEFAULT_API_BASE.to_owned(), str::to_owned),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}

fn header_pairs(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::key_before_auth_token(0)]
#[case::base_env_precedence(1)]
#[case::oauth_key_prefix(2)]
#[case::oauth_existing_authorization(3)]
#[case::custom_base_bearer_opt_in(4)]
#[case::missing_credential(5)]
#[case::models_http_fetch(6)]
#[case::models_non_2xx(7)]
#[case::oauth_direct_key_headers(8)]
#[case::models_response_limit(9)]
#[tokio::test]
async fn credentials_cases(#[case] case: u8) {
    match case {
        0 => key_before_auth_token_case(),
        1 => base_env_precedence_case(),
        2 => {
            oauth_key_prefix_detection();
            oauth_token_claim_projection();
        }
        3 => {
            oauth_existing_authorization_extracts();
            oauth_existing_authorization_first_key_wins();
        }
        4 => {
            custom_base_bearer_default();
            custom_base_bearer_lookalikes();
        }
        5 => missing_credential_case().await,
        6 => models_http_fetch_case().await,
        7 => {
            models_status_errors().await;
            models_json_errors().await;
            models_transport_failure().await;
            anthropic_error_literal();
        }
        8 => {
            oauth_direct_key_rewrites_headers();
            oauth_direct_key_ignores_prefixed_and_plain();
        }
        _ => models_response_limit_case().await,
    }
}

fn key_before_auth_token_case() {
    let environment = configured(Some("env-key"), Some("auth-token-value"), None);
    let env_header = get_auth_header(None, None, false, &environment).unwrap();
    assert_eq!(env_header.name, "x-api-key");
    assert_eq!(env_header.value.expose_secret(), "env-key");

    let explicit =
        get_auth_header(Some(&secret("explicit-key")), None, false, &environment).unwrap();
    assert_eq!(explicit.value.expose_secret(), "explicit-key");

    let empty = configured(Some(""), Some("auth-token-value"), None);
    let empty_header = get_auth_header(None, None, false, &empty).unwrap();
    assert_eq!(empty_header.name, "x-api-key");
    assert_eq!(empty_header.value.expose_secret(), "");

    let oauth = configured(Some("sk-ant-oat-token"), None, None);
    let oauth_header = get_auth_header(None, None, false, &oauth).unwrap();
    assert_eq!(oauth_header.name, "authorization");
    assert_eq!(
        oauth_header.value.expose_secret(),
        "Bearer sk-ant-oat-token"
    );

    let prefixed = configured(Some("Bearer sk-ant-oat-token"), None, None);
    let prefixed_header = get_auth_header(None, None, false, &prefixed).unwrap();
    assert_eq!(
        prefixed_header.value.expose_secret(),
        "Bearer Bearer sk-ant-oat-token"
    );

    let token_only = configured(None, Some("auth-token-value"), None);
    let token_header = get_auth_header(None, None, false, &token_only).unwrap();
    assert_eq!(token_header.name, "authorization");
    assert_eq!(
        token_header.value.expose_secret(),
        "Bearer auth-token-value"
    );
}

fn base_env_precedence_case() {
    let environment = configured(Some("key"), None, Some("https://env.example"));
    assert_eq!(
        models_request(None, None, &environment).unwrap().url,
        "https://env.example/v1/models"
    );
    assert_eq!(
        models_request(None, Some("https://explicit.example"), &environment)
            .unwrap()
            .url,
        "https://explicit.example/v1/models"
    );
    assert_eq!(
        models_request(None, None, &configured(Some("key"), None, None))
            .unwrap()
            .url,
        "https://api.anthropic.com/v1/models"
    );
    assert_eq!(
        get_base_model(Some("anthropic/anthropic/claude-x")),
        Some("claude-x".to_owned())
    );
    assert_eq!(
        get_base_model(Some("ANTHROPIC/claude-x")),
        Some("ANTHROPIC/claude-x".to_owned())
    );
    assert_eq!(get_base_model(None), None);
    assert_eq!(get_base_model(Some("")), None);
}

fn oauth_key_prefix_detection() {
    assert!(is_anthropic_oauth_key(Some("sk-ant-oat-token")));
    assert!(is_anthropic_oauth_key(Some("Bearer sk-ant-oat-token")));
    assert!(!is_anthropic_oauth_key(Some("bearer sk-ant-oat-token")));
    assert!(!is_anthropic_oauth_key(Some("Bearer sk-ant-API-token")));
    assert!(!is_anthropic_oauth_key(Some("sk-ant-api")));
    assert!(!is_anthropic_oauth_key(Some("")));
    assert!(!is_anthropic_oauth_key(None));
    assert!(!is_anthropic_oauth_key(Some(
        "BearerBearer sk-ant-oat-token"
    )));
}

fn oauth_token_claim_projection() {
    assert_eq!(
        resolve_oauth_token_claim(OauthTokenClaim::Boolean(true), Some("anthropic")),
        Some(true)
    );
    assert_eq!(
        resolve_oauth_token_claim(OauthTokenClaim::Boolean(true), Some("azure_ai")),
        Some(false)
    );
    assert_eq!(
        resolve_oauth_token_claim(OauthTokenClaim::Boolean(false), Some("anthropic")),
        Some(false)
    );
    assert_eq!(
        resolve_oauth_token_claim(OauthTokenClaim::Boolean(true), None),
        Some(false)
    );
    assert_eq!(
        resolve_oauth_token_claim(OauthTokenClaim::Other(IgnoredAny), Some("anthropic")),
        None
    );
    assert!(matches!(
        serde_json::from_str::<OauthTokenClaim>("\"yes\"").unwrap(),
        OauthTokenClaim::Other(_)
    ));
    assert!(matches!(
        serde_json::from_str::<OauthTokenClaim>("1").unwrap(),
        OauthTokenClaim::Other(_)
    ));
    assert!(matches!(
        serde_json::from_str::<OauthTokenClaim>("true").unwrap(),
        OauthTokenClaim::Boolean(true)
    ));
}

fn oauth_existing_authorization_extracts() {
    let mut headers = header_pairs(&[
        ("Authorization", "Bearer sk-ant-oat-token"),
        ("x-api-key", "stale-key"),
        ("anthropic-beta", " beta-b ,alpha ,, beta-b "),
    ]);
    let resolved = handle_anthropic_oauth(&mut headers, Some(secret("raw-key")));
    assert_eq!(revealed(resolved.as_ref()), Some("sk-ant-oat-token"));
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some("Bearer sk-ant-oat-token")
    );
    assert!(!headers.contains_key("Authorization"));
    assert!(!headers.contains_key("x-api-key"));
    assert_eq!(
        headers.get(OAUTH_BETA_HEADER_NAME).map(String::as_str),
        Some("alpha,beta-b,oauth-2025-04-20")
    );
    assert_eq!(
        headers.get(BROWSER_ACCESS_HEADER).map(String::as_str),
        Some("true")
    );
}

fn oauth_existing_authorization_first_key_wins() {
    let mut blocked = header_pairs(&[
        ("Authorization", "Bearer plain"),
        ("authorization", "Bearer sk-ant-oat-later"),
    ]);
    let blocked_key = handle_anthropic_oauth(&mut blocked, None);
    assert!(blocked_key.is_none());
    assert_eq!(
        blocked.get("Authorization").map(String::as_str),
        Some("Bearer plain")
    );
    assert_eq!(
        blocked.get("authorization").map(String::as_str),
        Some("Bearer sk-ant-oat-later")
    );

    let mut mixed = header_pairs(&[("AUTHORIZATION", "Bearer sk-ant-oat-mixed")]);
    let mixed_key = handle_anthropic_oauth(&mut mixed, None);
    assert_eq!(revealed(mixed_key.as_ref()), Some("sk-ant-oat-mixed"));
    assert_eq!(
        mixed.get("authorization").map(String::as_str),
        Some("Bearer sk-ant-oat-mixed")
    );
    assert!(!mixed.contains_key("AUTHORIZATION"));
    assert_eq!(
        mixed.get(OAUTH_BETA_HEADER_NAME).map(String::as_str),
        Some(OAUTH_BETA_HEADER)
    );
}

fn custom_base_bearer_default() {
    let key = secret("sk-custom-key");
    let default = make_api_key_auth_header(&key, Some("https://custom.example"), false);
    assert_eq!(default.name, "x-api-key");
    assert_eq!(default.value.expose_secret(), "sk-custom-key");

    let opt_in = make_api_key_auth_header(&key, Some("https://custom.example"), true);
    assert_eq!(opt_in.name, "authorization");
    assert_eq!(opt_in.value.expose_secret(), "Bearer sk-custom-key");

    let prefixed = make_api_key_auth_header(
        &secret("Bearer sk-custom-key"),
        Some("https://custom.example"),
        true,
    );
    assert_eq!(prefixed.value.expose_secret(), "Bearer sk-custom-key");

    let no_base = make_api_key_auth_header(&key, None, true);
    assert_eq!(no_base.name, "x-api-key");
}

fn custom_base_bearer_lookalikes() {
    let key = secret("sk-custom-key");
    for base in [
        "https://api.anthropic.com",
        "https://proxy.example/api.anthropic.com",
        "https://api.anthropic.com.evil.example",
        "",
    ] {
        let lookalike = make_api_key_auth_header(&key, Some(base), true);
        assert_eq!(lookalike.name, "x-api-key", "{base}");
    }

    let official = make_api_key_auth_header(
        &secret("sk-ant-oat-key"),
        Some("https://custom.example"),
        true,
    );
    assert_eq!(official.name, "x-api-key");
}

async fn missing_credential_case() {
    let script = Script::replying(200, b"{}");
    let error = get_models(&script, None, None, &AnthropicSettings::default(), DEADLINE)
        .await
        .unwrap_err();
    assert!(matches!(error, CredentialsError::MissingCredential));
    let message = error.to_string();
    assert!(message.contains(
        "ANTHROPIC_API_BASE/ANTHROPIC_BASE_URL or ANTHROPIC_API_KEY/ANTHROPIC_AUTH_TOKEN is not set"
    ));
    assert!(message.contains("to query Anthropic's `/models` endpoint."));
    assert!(script.requests().is_empty());

    let empty_key = configured(Some(""), None, None);
    let request = models_request(None, None, &empty_key).unwrap();
    assert_eq!(request.auth.name, "x-api-key");
    assert_eq!(request.auth.value.expose_secret(), "");
    assert_eq!(request.anthropic_version, ANTHROPIC_VERSION);
}

async fn models_http_fetch_case() {
    models_http_fetch_custom_base().await;
    models_http_fetch_oauth_and_explicit().await;
}

async fn models_http_fetch_custom_base() {
    let custom = configured(Some("api-key"), None, Some("https://custom.example/"));
    let script = Script::replying(
        200,
        br#"{"data":[{"id":"claude-x"},{"id":"anthropic/claude-y"}],"has_more":false}"#,
    );
    let models = get_models(&script, None, None, &custom, DEADLINE)
        .await
        .unwrap();
    assert_eq!(
        models,
        vec![
            "anthropic/claude-x".to_owned(),
            "anthropic/anthropic/claude-y".to_owned()
        ]
    );
    let sent = script.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::GET);
    assert_eq!(sent[0].url, "https://custom.example//v1/models");
    assert_eq!(
        sent[0].headers,
        vec![
            ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
            ("x-api-key".to_owned(), "api-key".to_owned())
        ]
    );
    assert_eq!(sent[0].sensitive, vec!["x-api-key".to_owned()]);
}

async fn models_http_fetch_oauth_and_explicit() {
    let custom = configured(Some("api-key"), None, Some("https://custom.example/"));
    let oauth = configured(Some("sk-ant-oat-model"), None, None);
    let oauth_script = Script::replying(299, br#"{"data":[{"id":""}]}"#);
    let boundary = get_models(&oauth_script, None, None, &oauth, DEADLINE)
        .await
        .unwrap();
    assert_eq!(boundary, vec!["anthropic/".to_owned()]);
    let oauth_sent = oauth_script.requests();
    assert_eq!(oauth_sent[0].url, "https://api.anthropic.com/v1/models");
    assert_eq!(
        oauth_sent[0].headers,
        vec![
            ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
            (
                "authorization".to_owned(),
                "Bearer sk-ant-oat-model".to_owned()
            )
        ]
    );
    assert_eq!(oauth_sent[0].sensitive, vec!["authorization".to_owned()]);

    let explicit = Script::replying(200, br#"{"data":[]}"#);
    let empty = get_models(
        &explicit,
        Some(&secret("explicit-key")),
        Some("https://explicit.example"),
        &custom,
        DEADLINE,
    )
    .await
    .unwrap();
    assert!(empty.is_empty());
    let explicit_sent = explicit.requests();
    assert_eq!(explicit_sent[0].url, "https://explicit.example/v1/models");
    assert_eq!(
        explicit_sent[0].headers[1],
        ("x-api-key".to_owned(), "explicit-key".to_owned())
    );
}

async fn models_status_errors() {
    let settings = configured(Some("api-key"), None, None);
    let slow = Script::replying(429, b"slow down");
    let status_error = get_models(&slow, None, None, &settings, DEADLINE)
        .await
        .unwrap_err();
    assert_eq!(
        status_error.to_string(),
        "Failed to fetch models from Anthropic. Status code: 429, Response: slow down"
    );
    let redirect = Script::replying(300, b"redirect");
    let redirect_error = get_models(&redirect, None, None, &settings, DEADLINE)
        .await
        .unwrap_err();
    assert!(redirect_error.to_string().contains("Status code: 300"));
    let binary = Script::replying(400, &[0xff, 0xfe]);
    let binary_error = get_models(&binary, None, None, &settings, DEADLINE)
        .await
        .unwrap_err();
    assert!(
        binary_error
            .to_string()
            .contains("2 bytes that are not UTF-8")
    );
}

async fn assert_models_json_error(status: u16, body: &[u8]) {
    let settings = configured(Some("api-key"), None, None);
    let script = Script::replying(status, body);
    let error = get_models(&script, None, None, &settings, DEADLINE)
        .await
        .unwrap_err();
    assert!(
        matches!(error, CredentialsError::ModelsJson { .. }),
        "{status} {body:?}"
    );
}

async fn models_json_errors() {
    assert_models_json_error(200, b"{\"data\": [").await;
    assert_models_json_error(200, br#"{"data":[{"other":"x"}]}"#).await;
    assert_models_json_error(200, b"{}").await;
    assert_models_json_error(204, b"").await;
}

async fn models_transport_failure() {
    let settings = configured(Some("api-key"), None, None);
    let script = Script::failing();
    let error = get_models(&script, None, None, &settings, DEADLINE)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CredentialsError::Transport {
            source: TransportError::Closed
        }
    ));
    assert_eq!(script.requests().len(), 1);

    let invalid_key = configured(Some("line\nbreak"), None, None);
    let unsent = Script::replying(200, b"{}");
    let invalid = get_models(&unsent, None, None, &invalid_key, DEADLINE)
        .await
        .unwrap_err();
    assert!(matches!(
        invalid,
        CredentialsError::InvalidHeader { name: "x-api-key" }
    ));

    let bad_base = configured(Some("api-key"), None, Some("not a url"));
    let bad = get_models(&unsent, None, None, &bad_base, DEADLINE)
        .await
        .unwrap_err();
    assert!(matches!(bad, CredentialsError::InvalidUrl { .. }));
    assert!(unsent.requests().is_empty());
}

fn padded_model_list(total: usize) -> Vec<u8> {
    let mut body = br#"{"data":[]}"#.to_vec();
    body.resize(total, b' ');
    body
}

async fn models_response_limit_case() {
    assert_eq!(MODEL_LIST_LIMIT, 0x0040_0000);
    let at_limit = padded_model_list(MODEL_LIST_LIMIT);
    assert!(decode_anthropic_models(200, &at_limit).unwrap().is_empty());
    let over_limit = padded_model_list(MODEL_LIST_LIMIT + 1);
    let too_large = decode_anthropic_models(200, &over_limit).unwrap_err();
    assert!(matches!(
        too_large,
        CredentialsError::ResponseTooLarge {
            limit: MODEL_LIST_LIMIT
        }
    ));
    assert_eq!(
        too_large.to_string(),
        format!("Anthropic models response is larger than {MODEL_LIST_LIMIT} bytes")
    );
    let failure_over_limit = decode_anthropic_models(500, &over_limit).unwrap_err();
    assert!(matches!(
        failure_over_limit,
        CredentialsError::ResponseTooLarge { .. }
    ));
    let at_limit_failure = decode_anthropic_models(500, &at_limit).unwrap_err();
    assert!(matches!(
        at_limit_failure,
        CredentialsError::ModelsStatus { status: 500, .. }
    ));

    let settings = configured(Some("api-key"), None, None);
    let script = Script::replying(200, &over_limit);
    let fetched = get_models(&script, None, None, &settings, DEADLINE)
        .await
        .unwrap_err();
    assert!(matches!(fetched, CredentialsError::ResponseTooLarge { .. }));
}

fn anthropic_error_literal() {
    let projected = ProviderError::from(ProviderErrorParts {
        status_code: 429,
        message: "slow".to_owned(),
        headers: Some(vec![("retry-after".to_owned(), "2".to_owned())]),
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    });
    assert_eq!(projected.status_code, 429);
    assert_eq!(projected.message, "slow");
    assert_eq!(
        projected.headers,
        Some(vec![("retry-after".to_owned(), "2".to_owned())])
    );
    assert!(!projected.status_code_is_synthesized);
}

fn oauth_direct_key_rewrites_headers() {
    let mut headers = header_pairs(&[
        ("x-api-key", "lower-stale"),
        ("X-Api-Key", "upper-stale"),
        ("Authorization", "Bearer kept"),
    ]);
    let resolved = handle_anthropic_oauth(&mut headers, Some(secret("sk-ant-oat-raw")));
    assert_eq!(revealed(resolved.as_ref()), Some("sk-ant-oat-raw"));
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some("Bearer sk-ant-oat-raw")
    );
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer kept")
    );
    assert!(!headers.contains_key("x-api-key"));
    assert!(!headers.contains_key("X-Api-Key"));
    assert_eq!(
        headers.get(OAUTH_BETA_HEADER_NAME).map(String::as_str),
        Some(OAUTH_BETA_HEADER)
    );
    assert_eq!(
        headers.get(BROWSER_ACCESS_HEADER).map(String::as_str),
        Some("true")
    );
}

fn oauth_direct_key_ignores_prefixed_and_plain() {
    let mut prefixed = header_pairs(&[("x-api-key", "stale")]);
    let prefixed_key = handle_anthropic_oauth(&mut prefixed, Some(secret("Bearer sk-ant-oat-x")));
    assert_eq!(revealed(prefixed_key.as_ref()), Some("Bearer sk-ant-oat-x"));
    assert_eq!(prefixed.get("x-api-key").map(String::as_str), Some("stale"));
    assert!(!prefixed.contains_key("authorization"));

    let mut plain = header_pairs(&[("x-api-key", "kept")]);
    let plain_key = handle_anthropic_oauth(&mut plain, Some(secret("plain-key")));
    assert_eq!(revealed(plain_key.as_ref()), Some("plain-key"));
    assert_eq!(plain, header_pairs(&[("x-api-key", "kept")]));

    let mut empty_beta = header_pairs(&[("anthropic-beta", "")]);
    let beta_key = handle_anthropic_oauth(&mut empty_beta, Some(secret("sk-ant-oat-y")));
    assert_eq!(revealed(beta_key.as_ref()), Some("sk-ant-oat-y"));
    assert_eq!(
        empty_beta.get(OAUTH_BETA_HEADER_NAME).map(String::as_str),
        Some(OAUTH_BETA_HEADER)
    );
}

#[test]
fn anthropic_credentials_credential_identity_isolation() {
    let first = configured(
        Some("sk-first-identity"),
        Some("token-first"),
        Some("https://first.example"),
    );
    let second = configured(
        Some("sk-second-identity"),
        Some("token-second"),
        Some("https://second.example"),
    );
    let first_header = get_auth_header(None, None, false, &first).unwrap();
    let second_header = get_auth_header(None, None, false, &second).unwrap();
    assert_eq!(first_header.value.expose_secret(), "sk-first-identity");
    assert_eq!(second_header.value.expose_secret(), "sk-second-identity");
    assert_ne!(
        first_header.value.expose_secret(),
        second_header.value.expose_secret()
    );
    assert_eq!(
        get_auth_header(None, None, false, &first)
            .unwrap()
            .value
            .expose_secret(),
        "sk-first-identity"
    );

    let oauth = configured(Some("sk-ant-oat-first"), None, None);
    let mut headers = header_pairs(&[("x-api-key", "stale")]);
    let resolved = handle_anthropic_oauth(&mut headers, oauth.api_key);
    assert_eq!(revealed(resolved.as_ref()), Some("sk-ant-oat-first"));
    assert!(
        !headers
            .values()
            .any(|value| value.contains("sk-second-identity"))
    );
}

#[tokio::test]
async fn anthropic_credentials_secret_safe_failure() {
    let settings = configured(
        Some("sk-secret-fixture-key"),
        Some("secret-fixture-token"),
        Some("https://secret.example"),
    );
    let debug = format!("{settings:?}");
    assert!(!debug.contains("sk-secret-fixture-key"));
    assert!(!debug.contains("secret-fixture-token"));

    let header = get_auth_header(None, None, false, &settings).unwrap();
    assert!(!format!("{header:?}").contains("sk-secret-fixture-key"));
    let oauth_header = make_api_key_auth_header(&secret("sk-ant-oat-secret-fixture"), None, false);
    assert!(!format!("{oauth_header:?}").contains("sk-ant-oat-secret-fixture"));

    let request = models_request(None, None, &settings).unwrap();
    let request_debug = format!("{request:?}");
    assert!(request_debug.contains("https://secret.example/v1/models"));
    assert!(!request_debug.contains("sk-secret-fixture-key"));
    let outbound = request.into_outbound(DEADLINE).unwrap();
    assert!(!format!("{outbound:?}").contains("sk-secret-fixture-key"));

    let script = Script::replying(401, b"denied");
    let error = get_models(&script, None, None, &settings, DEADLINE)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CredentialsError::ModelsStatus { status: 401, .. }
    ));
    assert!(!format!("{error:?}").contains("sk-secret-fixture-key"));
    assert!(!error.to_string().contains("sk-secret-fixture-key"));
    assert_eq!(
        script.requests()[0].headers[1],
        ("x-api-key".to_owned(), "sk-secret-fixture-key".to_owned())
    );

    let missing = models_request(None, None, &AnthropicSettings::default()).unwrap_err();
    assert!(!format!("{missing:?}").contains("sk-secret-fixture"));
    assert!(!missing.to_string().contains("sk-secret-fixture"));
}

#[test]
fn replays_the_anthropic_credentials_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_credentials");
    let sources = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap()
                .to_owned();
            (name, fs::read(path).unwrap())
        })
        .collect::<Vec<_>>();
    assert!(sources.len() >= 4);
    assert!(sources.iter().all(|(name, bytes)| {
        let failed = matches!(
            decode_anthropic_models(500, bytes),
            Err(CredentialsError::ModelsStatus { status: 500, .. })
        );
        let success = decode_anthropic_models(200, bytes);
        let expected = match name.as_str() {
            "valid.json" | "boundary.json" => success.is_ok(),
            "malformed.json" | "truncated.json" => success.is_err(),
            _ => true,
        };
        failed && expected
    }));
    let (_, valid) = sources
        .iter()
        .find(|(name, _)| name == "valid.json")
        .unwrap();
    let models = decode_anthropic_models(200, valid).unwrap();
    assert_eq!(
        models.first().map(String::as_str),
        Some("anthropic/claude-3-5-sonnet-20241022")
    );
}

#[tokio::test]
async fn models_go_over_the_shared_client_on_the_wire() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let body = br#"{"data":[{"id":"claude-wire"}]}"#;
    let server = async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        let mut byte = [0_u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        let reply = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(reply.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
        stream.shutdown().await.unwrap();
        String::from_utf8(head).unwrap()
    };
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    let settings = configured(Some("wire-key"), None, Some(&base));
    let (head, models) = tokio::join!(server, get_models(&client, None, None, &settings, DEADLINE));
    assert_eq!(models.unwrap(), vec!["anthropic/claude-wire".to_owned()]);
    assert!(head.starts_with("GET /v1/models HTTP/1.1\r\n"));
    assert!(head.contains("x-api-key: wire-key\r\n"));
    assert!(head.contains("anthropic-version: 2023-06-01\r\n"));
}
