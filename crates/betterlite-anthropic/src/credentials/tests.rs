#![cfg(test)]

use std::fs;
use std::path::Path;

use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use serde::de::IgnoredAny;

use super::{
    ANTHROPIC_VERSION, AnthropicSettings, BROWSER_ACCESS_HEADER, CredentialsError,
    DEFAULT_API_BASE, OAUTH_BETA_HEADER, OAUTH_BETA_HEADER_NAME, OauthTokenClaim, anthropic_error,
    decode_anthropic_models, get_api_base, get_auth_header, get_base_model, handle_anthropic_oauth,
    is_anthropic_oauth_key, make_api_key_auth_header, models_request, resolve_oauth_token_claim,
};

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
        api_base: api_base.map(str::to_owned),
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
fn credentials_cases(#[case] case: u8) {
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
        5 => missing_credential_case(),
        6 => models_http_fetch_case(),
        7 => {
            models_status_errors();
            models_json_errors();
            anthropic_error_projection();
        }
        _ => {
            oauth_direct_key_rewrites_headers();
            oauth_direct_key_ignores_prefixed_and_plain();
        }
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
    let environment = configured(None, None, Some("https://env.example"));
    assert_eq!(get_api_base(None, &environment), "https://env.example");
    assert_eq!(
        get_api_base(Some("https://explicit.example"), &environment),
        "https://explicit.example"
    );
    assert_eq!(
        get_api_base(None, &AnthropicSettings::default()),
        DEFAULT_API_BASE
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

fn missing_credential_case() {
    let error = models_request(None, None, &AnthropicSettings::default()).unwrap_err();
    let message = error.to_string();
    assert!(message.contains(
        "ANTHROPIC_API_BASE/ANTHROPIC_BASE_URL or ANTHROPIC_API_KEY/ANTHROPIC_AUTH_TOKEN is not set"
    ));
    assert!(message.contains("to query Anthropic's `/models` endpoint."));

    let empty_key = configured(Some(""), None, None);
    let request = models_request(None, None, &empty_key).unwrap();
    assert_eq!(
        request.headers.get("x-api-key").map(String::as_str),
        Some("")
    );
    assert_eq!(
        request.headers.get("anthropic-version").map(String::as_str),
        Some(ANTHROPIC_VERSION)
    );
}

fn models_http_fetch_case() {
    let custom = configured(Some("api-key"), None, Some("https://custom.example/"));
    let custom_request = models_request(None, None, &custom).unwrap();
    assert_eq!(custom_request.url, "https://custom.example//v1/models");
    assert_eq!(
        custom_request
            .headers
            .get("anthropic-version")
            .map(String::as_str),
        Some("2023-06-01")
    );
    assert_eq!(
        custom_request.headers.get("x-api-key").map(String::as_str),
        Some("api-key")
    );
    assert!(!custom_request.headers.contains_key(OAUTH_BETA_HEADER_NAME));
    assert!(!custom_request.headers.contains_key(BROWSER_ACCESS_HEADER));

    let oauth = configured(Some("sk-ant-oat-model"), None, None);
    let oauth_request = models_request(None, None, &oauth).unwrap();
    assert_eq!(oauth_request.url, "https://api.anthropic.com/v1/models");
    assert_eq!(
        oauth_request
            .headers
            .get("authorization")
            .map(String::as_str),
        Some("Bearer sk-ant-oat-model")
    );

    let decoded = decode_anthropic_models(
        200,
        br#"{"data":[{"id":"claude-x"},{"id":"anthropic/claude-y"}],"has_more":false}"#,
    )
    .unwrap();
    assert_eq!(
        decoded,
        vec![
            "anthropic/claude-x".to_owned(),
            "anthropic/anthropic/claude-y".to_owned()
        ]
    );
    let boundary = decode_anthropic_models(299, br#"{"data":[{"id":""}]}"#).unwrap();
    assert_eq!(boundary, vec!["anthropic/".to_owned()]);
}

fn models_status_errors() {
    let status_error = decode_anthropic_models(429, b"slow down").unwrap_err();
    assert_eq!(
        status_error.to_string(),
        "Failed to fetch models from Anthropic. Status code: 429, Response: slow down"
    );
    let redirect_error = decode_anthropic_models(300, b"redirect").unwrap_err();
    assert!(redirect_error.to_string().contains("Status code: 300"));
    let binary_error = decode_anthropic_models(400, &[0xff, 0xfe]).unwrap_err();
    assert!(
        binary_error
            .to_string()
            .contains("2 bytes that are not UTF-8")
    );
}

fn models_json_errors() {
    let malformed = decode_anthropic_models(200, b"{\"data\": [").unwrap_err();
    assert!(matches!(malformed, CredentialsError::ModelsJson { .. }));
    let missing_id = decode_anthropic_models(200, br#"{"data":[{"other":"x"}]}"#).unwrap_err();
    assert!(matches!(missing_id, CredentialsError::ModelsJson { .. }));
    let missing_data = decode_anthropic_models(200, b"{}").unwrap_err();
    assert!(matches!(missing_data, CredentialsError::ModelsJson { .. }));
    let empty_body = decode_anthropic_models(204, b"").unwrap_err();
    assert!(matches!(empty_body, CredentialsError::ModelsJson { .. }));
}

fn anthropic_error_projection() {
    let projected = anthropic_error(
        429,
        "slow".to_owned(),
        Some(vec![("retry-after".to_owned(), "2".to_owned())]),
    );
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

#[test]
fn anthropic_credentials_secret_safe_failure() {
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

    let error = models_request(None, None, &AnthropicSettings::default()).unwrap_err();
    assert!(!format!("{error:?}").contains("sk-secret-fixture"));
    assert!(!error.to_string().contains("sk-secret-fixture"));

    let projected = anthropic_error(401, "denied".to_owned(), None);
    assert!(!format!("{projected:?}").contains("sk-secret-fixture"));
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
    assert!(sources.iter().any(|(name, bytes)| {
        name == "valid.json" && decode_anthropic_models(200, bytes).is_ok()
    }));
    assert!(sources.iter().any(|(name, bytes)| {
        name == "boundary.json" && decode_anthropic_models(200, bytes).is_ok()
    }));
    assert!(sources.iter().any(|(name, bytes)| {
        name == "malformed.json" && decode_anthropic_models(200, bytes).is_err()
    }));
    assert!(sources.iter().any(|(name, bytes)| {
        name == "truncated.json" && decode_anthropic_models(200, bytes).is_err()
    }));
    assert!(sources.iter().any(|(name, bytes)| {
        name == "valid.json" && decode_anthropic_models(500, bytes).is_err()
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
