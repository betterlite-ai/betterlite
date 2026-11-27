#![cfg(test)]

use std::fs;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_db::budget_permission_rows::JsonData;
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use rstest::rstest;

use super::check::{CheckError, ValidationError, check_oauth2_token};
use super::proxy_identity::{ProxyAuthError, handle_oauth2_proxy_request};
use super::request::EndpointMode;
use super::request::endpoint_mode as request_mode;
use super::{
    DecodeError, MAX_OAUTH_RESPONSE_BYTES, MAX_OAUTH_RESPONSE_DEPTH, decode_oauth_response,
};
use crate::resolved_key::auth::AuthError;
use crate::trusted_network::proxy_utils::TrustedProxyRequirementError;
use crate::trusted_network::settings::TrustedNetworkSettings;
use crate::user_roles::UserRole;

mod support;

use support::{
    CLIENT_SECRET, ENDPOINT, Logs, Scripted, TOKEN, capture, events, headers, mappings, network,
    settings, token, trusted,
};

const TRUSTED_PEER: Option<&str> = Some("10.0.0.5");
const INTROSPECTION_BODY: &str = r#"{"sub":"user-1","role":"internal_user","team_id":"team-7"}"#;

fn expect_validation(error: CheckError) -> ValidationError {
    match error {
        CheckError::Validation { source } => source,
        other => panic!("expected a validation failure, got {other}"),
    }
}

#[rstest]
#[case(None, None, None, EndpointMode::TokenInfo)]
#[case(
    Some("https://idp.example/introspect"),
    Some("id"),
    Some("secret"),
    EndpointMode::Introspection
)]
#[case(
    Some("https://idp.example/INTROSPECT"),
    Some("id"),
    None,
    EndpointMode::TokenInfo
)]
#[case(
    Some("https://idp.example/introspect"),
    None,
    Some("secret"),
    EndpointMode::TokenInfo
)]
#[case(
    Some("https://idp.example/introspect"),
    Some(""),
    Some(""),
    EndpointMode::Introspection
)]
#[case(
    Some("https://idp.example/token-info"),
    Some("id"),
    Some("secret"),
    EndpointMode::TokenInfo
)]
#[case(Some(""), Some("id"), Some("secret"), EndpointMode::TokenInfo)]
fn endpoint_mode(
    #[case] endpoint: Option<&str>,
    #[case] client_id: Option<&str>,
    #[case] client_secret: Option<&str>,
    #[case] expected: EndpointMode,
) {
    let configured = settings(endpoint, client_id, client_secret);
    assert_eq!(request_mode(&configured), expected);
}

#[tokio::test]
async fn introspection_basic_post() {
    let configured = settings(Some(ENDPOINT), Some("client"), Some(CLIENT_SECRET));
    let transport = Scripted::answering(200, INTROSPECTION_BODY);
    let auth = check_oauth2_token(&configured, &transport, &token())
        .await
        .expect("the token validates");
    let request = transport.take();
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url.as_str(), ENDPOINT);
    assert_eq!(
        request.headers.get(CONTENT_TYPE).unwrap(),
        "application/x-www-form-urlencoded"
    );
    let encoded = STANDARD.encode(format!("client:{CLIENT_SECRET}"));
    let expected = format!("Basic {encoded}");
    assert_eq!(
        request.headers.get(AUTHORIZATION).unwrap(),
        expected.as_str()
    );
    assert_eq!(request.form, vec![("token".to_owned(), TOKEN.to_owned())]);
    assert_eq!(auth.view.token.user_id.as_deref(), Some("user-1"));
    assert_eq!(auth.extra.user_role, Some(UserRole::InternalUser));
    assert_eq!(auth.view.token.team_id.as_deref(), Some("team-7"));
}

#[tokio::test]
async fn token_info_bearer_get() {
    let configured = settings(Some("https://idp.example/userinfo"), None, None);
    let transport = Scripted::answering(200, r#"{"sub":"user-2"}"#);
    let auth = check_oauth2_token(&configured, &transport, &token())
        .await
        .expect("the token validates");
    let request = transport.take();
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.url.as_str(), "https://idp.example/userinfo");
    assert_eq!(
        request.headers.get(CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let bearer = format!("Bearer {TOKEN}");
    assert_eq!(request.headers.get(AUTHORIZATION).unwrap(), bearer.as_str());
    assert!(request.form.is_empty());
    assert_eq!(auth.view.token.user_id.as_deref(), Some("user-2"));
    assert_eq!(auth.view.token.team_id, None);
    assert_eq!(auth.extra.user_role, None);
}

#[tokio::test]
async fn empty_secret_public_client() {
    let configured = settings(Some(ENDPOINT), Some("client"), Some(""));
    let transport = Scripted::answering(200, r#"{"sub":"user-3"}"#);
    check_oauth2_token(&configured, &transport, &token())
        .await
        .expect("the token validates");
    let request = transport.take();
    assert_eq!(request.method, Method::POST);
    assert!(request.headers.get(AUTHORIZATION).is_none());
    assert_eq!(
        request.form,
        vec![
            ("token".to_owned(), TOKEN.to_owned()),
            ("client_id".to_owned(), "client".to_owned()),
        ]
    );
}

#[tokio::test]
async fn empty_client_sends_token() {
    let configured = settings(Some(ENDPOINT), Some(""), Some("secret"));
    let transport = Scripted::answering(200, r#"{"sub":"user-3"}"#);
    check_oauth2_token(&configured, &transport, &token())
        .await
        .expect("the token validates");
    let request = transport.take();
    assert_eq!(request.method, Method::POST);
    assert!(request.headers.get(AUTHORIZATION).is_none());
    assert_eq!(request.form, vec![("token".to_owned(), TOKEN.to_owned())]);
}

#[tokio::test]
async fn inactive_token() {
    let configured = settings(Some(ENDPOINT), Some("client"), Some("secret"));
    for body in [
        r#"{"active":false,"sub":"user-1"}"#,
        r#"{"active":null,"sub":"user-1"}"#,
        r#"{"active":0,"sub":"user-1"}"#,
        r#"{"active":0.0,"sub":"user-1"}"#,
        r#"{"active":"","sub":"user-1"}"#,
        r#"{"active":[],"sub":"user-1"}"#,
        r#"{"active":{},"sub":"user-1"}"#,
    ] {
        let transport = Scripted::answering(200, body);
        let error = check_oauth2_token(&configured, &transport, &token())
            .await
            .expect_err("an inactive token is refused");
        assert!(
            matches!(expect_validation(error), ValidationError::Inactive),
            "{body}"
        );
    }
    for body in [
        r#"{"active":true,"sub":"user-1"}"#,
        r#"{"active":1,"sub":"user-1"}"#,
        r#"{"active":-2,"sub":"user-1"}"#,
        r#"{"active":"yes","sub":"user-1"}"#,
        r#"{"active":[0],"sub":"user-1"}"#,
        r#"{"active":{"a":0},"sub":"user-1"}"#,
    ] {
        let transport = Scripted::answering(200, body);
        check_oauth2_token(&configured, &transport, &token())
            .await
            .expect("an active token validates");
    }
}

#[tokio::test]
async fn missing_active_default() {
    let configured = settings(Some(ENDPOINT), Some("client"), Some("secret"));
    let transport = Scripted::answering(200, r#"{"sub":"user-3"}"#);
    let auth = check_oauth2_token(&configured, &transport, &token())
        .await
        .expect("an absent active defaults to true");
    assert_eq!(auth.view.token.user_id.as_deref(), Some("user-3"));
}

#[tokio::test]
async fn get_ignores_active() {
    let configured = settings(Some("https://idp.example/userinfo"), None, None);
    let transport = Scripted::answering(200, r#"{"sub":"user-4","active":false}"#);
    let auth = check_oauth2_token(&configured, &transport, &token())
        .await
        .expect("a token info GET ignores the active key");
    assert_eq!(auth.view.token.user_id.as_deref(), Some("user-4"));
}

#[tokio::test]
async fn field_mappings() {
    let mut configured = settings(Some(ENDPOINT), None, None);
    configured.user_id_field_name = "user_id".to_owned();
    configured.user_role_field_name = "user_role".to_owned();
    configured.user_team_id_field_name = "team_id".to_owned();
    let transport = Scripted::answering(
        200,
        r#"{"user_id":"mapped-user","user_role":"proxy_admin","team_id":"mapped-team","sub":"ignored"}"#,
    );
    let auth = check_oauth2_token(&configured, &transport, &token())
        .await
        .expect("the mapped fields validate");
    assert_eq!(auth.view.token.user_id.as_deref(), Some("mapped-user"));
    assert_eq!(auth.extra.user_role, Some(UserRole::ProxyAdmin));
    assert_eq!(auth.view.token.team_id.as_deref(), Some("mapped-team"));

    // a configured name with a dot is one literal dictionary key, not a path into nested objects
    configured.user_id_field_name = "user.name".to_owned();
    let transport = Scripted::answering(200, r#"{"user":{"name":"nested"},"user.name":"literal"}"#);
    let auth = check_oauth2_token(&configured, &transport, &token())
        .await
        .expect("the literal dotted key validates");
    assert_eq!(auth.view.token.user_id.as_deref(), Some("literal"));
}

#[tokio::test]
async fn missing_identity_fields() {
    let configured = settings(Some(ENDPOINT), None, None);
    let transport = Scripted::answering(200, r#"{"active":true,"role":null}"#);
    let auth = check_oauth2_token(&configured, &transport, &token())
        .await
        .expect("an identity without the mapped fields validates");
    assert_eq!(auth.view.token.user_id, None);
    assert_eq!(auth.view.token.team_id, None);
    assert_eq!(auth.extra.user_role, None);
}

#[tokio::test]
async fn invalid_role() {
    let configured = settings(Some(ENDPOINT), None, None);
    let transport = Scripted::answering(200, r#"{"sub":"user-1","role":"superuser"}"#);
    let error = check_oauth2_token(&configured, &transport, &token())
        .await
        .expect_err("an unknown role is refused");
    let source = expect_validation(error);
    assert!(
        matches!(source, ValidationError::Auth(AuthError::View(_))),
        "{source}"
    );
    assert!(source.to_string().contains("verification token view"));
}

#[tokio::test]
async fn upstream_error() {
    let configured = settings(Some(ENDPOINT), Some("client"), Some("secret"));
    let status = check_oauth2_token(&configured, &Scripted::answering(401, "{}"), &token())
        .await
        .expect_err("a 401 is refused");
    assert_eq!(
        status.to_string(),
        "Oauth 2.0 Token validation failed: HTTP status 401"
    );
    let generic = check_oauth2_token(
        &configured,
        &Scripted::failing("connection refused"),
        &token(),
    )
    .await
    .expect_err("a network failure is refused");
    assert_eq!(
        generic.to_string(),
        "An error occurred during token validation: the token info request failed: connection refused"
    );
}

#[tokio::test]
async fn upstream_status_error() {
    let configured = settings(Some(ENDPOINT), Some("client"), Some("secret"));
    let transport = Scripted::answering(503, INTROSPECTION_BODY);
    let error = check_oauth2_token(&configured, &transport, &token())
        .await
        .expect_err("a 503 is refused even with a valid body");
    assert!(
        matches!(error, CheckError::Status { status: 503 }),
        "{error}"
    );
}

#[tokio::test]
async fn upstream_network_error() {
    let configured = settings(Some(ENDPOINT), Some("client"), Some("secret"));
    let error = check_oauth2_token(&configured, &Scripted::failing("dns error"), &token())
        .await
        .expect_err("a transport failure is refused");
    assert!(matches!(
        expect_validation(error),
        ValidationError::Transport(_)
    ));
}

#[tokio::test]
async fn invalid_response_json() {
    let configured = settings(Some(ENDPOINT), None, None);
    let error = check_oauth2_token(
        &configured,
        &Scripted::answering(200, "{\"sub\":"),
        &token(),
    )
    .await
    .expect_err("a malformed document is refused");
    assert!(matches!(
        expect_validation(error),
        ValidationError::Decode(DecodeError::Json(_))
    ));
}

#[tokio::test]
async fn endpoint_absent() {
    let transport = Scripted::answering(200, INTROSPECTION_BODY);
    for endpoint in [None, Some("")] {
        let configured = settings(endpoint, Some("client"), Some("secret"));
        let error = check_oauth2_token(&configured, &transport, &token())
            .await
            .expect_err("an absent endpoint is refused");
        assert!(matches!(error, CheckError::EndpointAbsent), "{endpoint:?}");
    }
    assert_eq!(
        transport.sent(),
        0,
        "the endpoint check precedes the network"
    );
}

#[tokio::test]
async fn oauth_without_license() {
    // the approved no-paid-tier deviation: a configured endpoint validates without a premium flag
    let configured = settings(Some(ENDPOINT), Some("client"), Some("secret"));
    let transport = Scripted::answering(200, INTROSPECTION_BODY);
    let auth = check_oauth2_token(&configured, &transport, &token())
        .await
        .expect("configured OAuth validates without an entitlement");
    assert_eq!(auth.view.token.user_id.as_deref(), Some("user-1"));
}

#[tokio::test]
async fn oauth_body_limits() {
    let oversized = format!("\"{}\"", "a".repeat(MAX_OAUTH_RESPONSE_BYTES));
    assert!(matches!(
        decode_oauth_response(oversized.as_bytes()),
        Err(DecodeError::TooLarge { .. })
    ));
    let boundary = format!(
        "{}{}",
        "[".repeat(MAX_OAUTH_RESPONSE_DEPTH),
        "]".repeat(MAX_OAUTH_RESPONSE_DEPTH)
    );
    assert!(decode_oauth_response(boundary.as_bytes()).is_ok());
    let over = format!(
        "{}{}",
        "[".repeat(MAX_OAUTH_RESPONSE_DEPTH + 1),
        "]".repeat(MAX_OAUTH_RESPONSE_DEPTH + 1)
    );
    assert!(matches!(
        decode_oauth_response(over.as_bytes()),
        Err(DecodeError::TooDeep { depth, limit })
            if depth == MAX_OAUTH_RESPONSE_DEPTH + 1 && limit == MAX_OAUTH_RESPONSE_DEPTH
    ));
    let configured = settings(Some(ENDPOINT), None, None);
    let transport = Scripted::answering(200, &oversized);
    let error = check_oauth2_token(&configured, &transport, &token())
        .await
        .expect_err("an oversized document is refused");
    assert!(matches!(
        expect_validation(error),
        ValidationError::Decode(DecodeError::TooLarge { .. })
    ));
}

#[tokio::test]
async fn oauth_secrets_redacted() {
    let logs = Logs::default();
    let guard = capture(&logs);
    let configured = settings(Some(ENDPOINT), Some("client"), Some(CLIENT_SECRET));
    let transport = Scripted::answering(200, r#"{"sub":"hidden-user"}"#);
    check_oauth2_token(&configured, &transport, &token())
        .await
        .expect("the token validates");
    drop(guard);
    let lines = events(&logs);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Using OAuth2 introspection endpoint (POST)")),
        "{lines:?}"
    );
    let joined = lines.join("\n");
    let credentials = STANDARD.encode(format!("client:{CLIENT_SECRET}"));
    for secret in [TOKEN, CLIENT_SECRET, credentials.as_str(), "hidden-user"] {
        assert!(
            !joined.contains(secret),
            "the log carries {secret}: {joined}"
        );
    }
}

#[test]
fn untrusted_proxy_headers() {
    let configured = trusted("10.0.0.0/8");
    let mapping = mappings(&[("user_id", "X-User-Id")]);
    let present = headers(&[("x-user-id", "user-1")]);
    let world = network(Some("203.0.113.7"), None);
    let error = handle_oauth2_proxy_request(&present, Some(&mapping), &world, &configured)
        .expect_err("an untrusted peer is refused");
    assert_eq!(
        error.to_string(),
        "OAuth2 proxy auth only accepts identity headers from configured trusted proxy ranges. Direct client IP '203.0.113.7' is not trusted."
    );
    let unknown = network(None, None);
    let error = handle_oauth2_proxy_request(&present, Some(&mapping), &unknown, &configured)
        .expect_err("a missing peer is refused");
    assert!(
        error
            .to_string()
            .contains("Direct client IP None is not trusted.")
    );
}

#[test]
fn proxy_trust_before_mapping() {
    let configured = trusted("10.0.0.0/8");
    let world = network(Some("203.0.113.7"), None);
    let present = headers(&[("x-user-id", "user-1")]);
    // the trust check runs before the allowlist, and before the mapping check
    let disallowed = mappings(&[("user_role", "X-User-Role")]);
    let error = handle_oauth2_proxy_request(&present, Some(&disallowed), &world, &configured)
        .expect_err("an untrusted peer is refused before the mapping is read");
    assert!(matches!(
        error,
        ProxyAuthError::Trust(TrustedProxyRequirementError::Untrusted { .. })
    ));
    let unconfigured = TrustedNetworkSettings::default();
    let peer = network(TRUSTED_PEER, None);
    let error = handle_oauth2_proxy_request(&present, None, &peer, &unconfigured)
        .expect_err("an unconfigured range set is refused before the missing mappings");
    assert!(matches!(
        error,
        ProxyAuthError::Trust(TrustedProxyRequirementError::NotConfigured { .. })
    ));
    assert_eq!(
        error.to_string(),
        "OAuth2 proxy auth requires general_settings.trusted_proxy_ranges before trusting identity headers from an upstream proxy."
    );
}

#[test]
fn proxy_mapping_absent() {
    let configured = trusted("10.0.0.0/8");
    let peer = network(TRUSTED_PEER, None);
    let present = headers(&[("x-user-id", "user-1")]);
    let empty = mappings(&[]);
    for configured_mappings in [None, Some(&empty)] {
        let error = handle_oauth2_proxy_request(&present, configured_mappings, &peer, &configured)
            .expect_err("an absent mapping refuses the request");
        assert!(matches!(error, ProxyAuthError::MappingsMissing));
        assert_eq!(
            error.to_string(),
            "Oauth2 config mappings not found in general_settings"
        );
    }
}

#[test]
fn proxy_user_role_mapping_rejected() {
    let configured = trusted("10.0.0.0/8");
    let peer = network(TRUSTED_PEER, None);
    let mapping = mappings(&[("user_id", "X-User-Id"), ("user_role", "X-User-Role")]);
    let present = headers(&[("x-user-id", "user-1"), ("x-user-role", "proxy_admin")]);
    let error = handle_oauth2_proxy_request(&present, Some(&mapping), &peer, &configured)
        .expect_err("a user_role mapping is refused");
    assert_eq!(
        error.to_string(),
        "Oauth2 proxy auth refuses to map non-identity UserAPIKeyAuth fields from request headers: \
         ['user_role']. Only identity fields are accepted \
         (['models', 'org_id', 'team_alias', 'team_id', 'user_email', 'user_id']); anything else \
         (privileges, budgets, rate limits, metadata) would let a caller forge enforcement \
         parameters by spoofing the matching header. If you need a trusted upstream to assert \
         anything beyond identity, use JWT auth (signature-validated) instead of header-trust."
    );
}

#[test]
fn proxy_disallowed_fields() {
    let configured = trusted("10.0.0.0/8");
    let peer = network(TRUSTED_PEER, None);
    let mapping = mappings(&[
        ("max_budget", "X-Max-Budget"),
        ("user_role", "X-User-Role"),
        ("api_key", "X-Api-Key"),
    ]);
    // no header needs to be present: the allowlist rejects the configuration before reading one
    let error = handle_oauth2_proxy_request(&headers(&[]), Some(&mapping), &peer, &configured)
        .expect_err("disallowed fields are refused");
    let message = error.to_string();
    assert!(
        message.contains("['api_key', 'max_budget', 'user_role']"),
        "{message}"
    );
    assert!(
        message
            .contains("(['models', 'org_id', 'team_alias', 'team_id', 'user_email', 'user_id'])"),
        "{message}"
    );
    assert!(matches!(error, ProxyAuthError::Disallowed { .. }));
}

#[test]
fn proxy_header_casefold() {
    let configured = trusted("10.0.0.0/8");
    let peer = network(TRUSTED_PEER, None);
    let mapping = mappings(&[("user_id", "X-User-Id")]);
    for name in ["x-user-id", "X-USER-ID"] {
        let present = headers(&[(name, "user-folded")]);
        let auth = handle_oauth2_proxy_request(&present, Some(&mapping), &peer, &configured)
            .expect("a case variant of the configured header maps");
        assert_eq!(auth.view.token.user_id.as_deref(), Some("user-folded"));
    }
}

#[test]
fn proxy_empty_header() {
    let configured = trusted("10.0.0.0/8");
    let peer = network(TRUSTED_PEER, None);
    let mapping = mappings(&[("user_id", "X-User-Id"), ("team_id", "X-Team-Id")]);
    let error = handle_oauth2_proxy_request(
        &headers(&[("x-user-id", "")]),
        Some(&mapping),
        &peer,
        &configured,
    )
    .expect_err("an empty header never maps");
    assert!(matches!(error, ProxyAuthError::NoIdentity));
    let auth = handle_oauth2_proxy_request(
        &headers(&[("x-user-id", "user-1"), ("x-team-id", "")]),
        Some(&mapping),
        &peer,
        &configured,
    )
    .expect("the present header maps and the empty one is skipped");
    assert_eq!(auth.view.token.user_id.as_deref(), Some("user-1"));
    assert_eq!(auth.view.token.team_id, None);
    let error = handle_oauth2_proxy_request(
        &headers(&[("x-other", "value")]),
        Some(&mapping),
        &peer,
        &configured,
    )
    .expect_err("no configured header maps to an identity");
    assert!(matches!(error, ProxyAuthError::NoIdentity));
}

#[test]
fn proxy_models_split() {
    let configured = trusted("10.0.0.0/8");
    let peer = network(TRUSTED_PEER, None);
    let mapping = mappings(&[("models", "X-Models")]);
    let auth = handle_oauth2_proxy_request(
        &headers(&[("x-models", " oauth-model , , other ")]),
        Some(&mapping),
        &peer,
        &configured,
    )
    .expect("the model list maps");
    assert_eq!(
        auth.view.token.models,
        vec![
            JsonData::String("oauth-model".to_owned()),
            JsonData::String(String::new()),
            JsonData::String("other".to_owned()),
        ]
    );
    let single = handle_oauth2_proxy_request(
        &headers(&[("x-models", "only")]),
        Some(&mapping),
        &peer,
        &configured,
    )
    .expect("a single model maps");
    assert_eq!(
        single.view.token.models,
        vec![JsonData::String("only".to_owned())]
    );
}

#[test]
fn proxy_all_identity_fields() {
    let logs = Logs::default();
    let guard = capture(&logs);
    let configured = trusted("10.0.0.0/8");
    let peer = network(TRUSTED_PEER, None);
    let mapping = mappings(&[
        ("user_id", "X-User-Id"),
        ("user_email", "X-User-Email"),
        ("team_id", "X-Team-Id"),
        ("team_alias", "X-Team-Alias"),
        ("org_id", "X-Org-Id"),
        ("models", "X-Models"),
    ]);
    let present = headers(&[
        ("x-user-id", "user-1"),
        ("x-user-email", "user@example.com"),
        ("x-team-id", "team-7"),
        ("x-team-alias", "team-alias-7"),
        ("x-org-id", "org-9"),
        ("x-models", "oauth-model"),
    ]);
    let auth = handle_oauth2_proxy_request(&present, Some(&mapping), &peer, &configured)
        .expect("all six identity fields map");
    drop(guard);
    assert_eq!(auth.view.token.user_id.as_deref(), Some("user-1"));
    assert_eq!(auth.extra.user_email.as_deref(), Some("user@example.com"));
    assert_eq!(auth.view.token.team_id.as_deref(), Some("team-7"));
    assert_eq!(auth.view.team_alias.as_deref(), Some("team-alias-7"));
    assert_eq!(auth.view.token.org_id.as_deref(), Some("org-9"));
    assert_eq!(
        auth.view.token.models,
        vec![JsonData::String("oauth-model".to_owned())]
    );
    assert_eq!(auth.extra.user_role, None);
    let joined = events(&logs).join("\n");
    for name in [
        "user_id",
        "user_email",
        "team_id",
        "team_alias",
        "org_id",
        "models",
    ] {
        assert!(
            joined.contains(name),
            "the diagnostics name {name}: {joined}"
        );
    }
    for value in [
        "user-1",
        "user@example.com",
        "team-alias-7",
        "org-9",
        "oauth-model",
    ] {
        assert!(
            !joined.contains(value),
            "the diagnostics leak {value}: {joined}"
        );
    }
}

#[test]
fn replays_the_oauth_tokens_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/oauth_tokens");
    let mut accepted = 0_usize;
    let mut deep = 0_usize;
    let mut malformed = 0_usize;
    for entry in fs::read_dir(corpus).expect("the corpus directory is readable") {
        let path = entry.expect("the corpus entry is readable").path();
        let bytes = fs::read(&path).expect("the corpus file is readable");
        match decode_oauth_response(&bytes) {
            Ok(_) => accepted += 1,
            Err(DecodeError::TooDeep { .. }) => deep += 1,
            Err(DecodeError::TooLarge { .. }) => {
                panic!(
                    "a committed corpus file is over the byte limit: {}",
                    path.display()
                );
            }
            Err(DecodeError::Json(_)) => malformed += 1,
        }
    }
    assert!(accepted >= 1, "the corpus carries a decodable document");
    assert!(deep >= 1, "the corpus crosses the depth limit");
    assert!(malformed >= 1, "the corpus carries malformed input");
}
