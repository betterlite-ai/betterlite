use rstest::rstest;

use crate::credential_contracts::auth_config::AuthConfig;
use crate::credential_contracts::auth_config::id_jag::{
    ClientAuth, ID_JAG_SUBJECT_TOKEN_TYPE, IdJagConfig,
};
use crate::credential_contracts::auth_config::key_source::{ApiKeyConfig, ApiKeySource};
use crate::credential_contracts::auth_config::oauth::{
    ClientCredentialsConfig, DEFAULT_SUBJECT_TOKEN_TYPE, TokenExchangeConfig, TokenExchangeProfile,
};
use crate::credential_contracts::auth_spec::AuthSpecKind;
use crate::credential_contracts::server_spec::ServerSpec;
use crate::credential_contracts::token_endpoint_auth::TokenEndpointAuthMethod;

use super::super::adapter::{UserPrincipal, to_server_spec, to_subject};
use super::support::{exposed, server_from};

const ID_JAG_BASE: &str = r#","auth_type":"oauth2_id_jag","token_exchange_endpoint":"https://idp.example.com/org/token","id_jag_resource_token_endpoint":"https://upstream.example.com/token","client_id":"client-1""#;

fn mapped(extra: &str) -> Option<ServerSpec> {
    let encoded = format!(
        r#"{{"server_id":"srv-1","name":"Server One","transport":"http","url":"https://mcp.example.com/mcp"{extra}}}"#
    );
    let record = server_from(&encoded);
    match to_server_spec(&record) {
        Ok(spec) => spec,
        Err(error) => panic!("adapter mapping failed: {error}"),
    }
}

fn owned(extra: &str) -> ServerSpec {
    mapped(extra).expect("the mode is owned by the resolver")
}

fn api_key(spec: &ServerSpec) -> &ApiKeyConfig {
    let AuthConfig::ApiKey(config) = &spec.config else {
        panic!("expected api_key, got {}", spec.config.kind());
    };
    config
}

fn shared_value(config: &ApiKeyConfig) -> String {
    match &config.key_source {
        ApiKeySource::Shared(shared) => exposed(&shared.value),
        ApiKeySource::Byok => panic!("expected a shared key source"),
    }
}

fn client_credentials(spec: &ServerSpec) -> &ClientCredentialsConfig {
    let AuthConfig::ClientCredentials(config) = &spec.config else {
        panic!("expected client_credentials, got {}", spec.config.kind());
    };
    config
}

fn token_exchange(spec: &ServerSpec) -> &TokenExchangeConfig {
    let AuthConfig::TokenExchange(config) = &spec.config else {
        panic!("expected token_exchange, got {}", spec.config.kind());
    };
    config
}

fn id_jag_config(extra: &str) -> IdJagConfig {
    let encoded = format!("{ID_JAG_BASE}{extra}");
    let spec = owned(&encoded);
    let AuthConfig::IdJag(config) = spec.config else {
        panic!("expected id_jag, got {}", spec.config.kind());
    };
    config
}

#[test]
fn to_subject_tenant_team_precedent() {
    let principal = UserPrincipal {
        org_id: Some("org-1".to_owned()),
        team_id: Some("team-1".to_owned()),
        user_id: Some("user-1".to_owned()),
    };
    let derived = to_subject(Some(&principal), Some("caller-token"));
    assert_eq!(derived.tenant_id, "org-1");
    assert_eq!(derived.subject_id, "user-1");
    assert_eq!(
        exposed(derived.inbound_token.as_ref().expect("inbound")),
        "caller-token"
    );
    let team_only = UserPrincipal {
        org_id: Some(String::new()),
        team_id: Some("team-1".to_owned()),
        user_id: None,
    };
    let extracted = to_subject(Some(&team_only), None);
    assert_eq!(extracted.tenant_id, "team-1");
    assert_eq!(extracted.subject_id, "");
    assert!(extracted.inbound_token.is_none());
}

#[test]
fn to_subject_empty_principal() {
    let anonymous = to_subject(None, Some("caller-token"));
    assert_eq!(anonymous.tenant_id, "");
    assert_eq!(anonymous.subject_id, "");
    assert_eq!(
        exposed(anonymous.inbound_token.as_ref().expect("inbound")),
        "caller-token"
    );
    let empty = UserPrincipal::default();
    let blank = to_subject(Some(&empty), Some(""));
    assert_eq!(blank.tenant_id, "");
    assert_eq!(blank.subject_id, "");
    assert!(blank.inbound_token.is_none());
}

#[rstest]
#[case::no_auth_type("", Some(AuthSpecKind::None))]
#[case::none(r#","auth_type":"none""#, Some(AuthSpecKind::None))]
#[case::api_key(
    r#","auth_type":"api_key","authentication_token":"shared""#,
    Some(AuthSpecKind::ApiKey)
)]
#[case::bearer_token(
    r#","auth_type":"bearer_token","authentication_token":"shared""#,
    Some(AuthSpecKind::ApiKey)
)]
#[case::token(
    r#","auth_type":"token","authentication_token":"shared""#,
    Some(AuthSpecKind::ApiKey)
)]
#[case::authorization(
    r#","auth_type":"authorization","authentication_token":"shared""#,
    Some(AuthSpecKind::ApiKey)
)]
#[case::basic(
    r#","auth_type":"basic","authentication_token":"shared""#,
    Some(AuthSpecKind::ApiKey)
)]
#[case::oauth2_user(r#","auth_type":"oauth2""#, Some(AuthSpecKind::AuthorizationCode))]
#[case::oauth2_delegated(r#","auth_type":"oauth2","delegate_auth_to_upstream":true"#, None)]
#[case::oauth2_client_credentials(
    r#","auth_type":"oauth2","oauth2_flow":"client_credentials""#,
    Some(AuthSpecKind::ClientCredentials)
)]
#[case::oauth2_client_credentials_delegated(
    r#","auth_type":"oauth2","oauth2_flow":"client_credentials","delegate_auth_to_upstream":true"#,
    Some(AuthSpecKind::ClientCredentials)
)]
#[case::oauth2_id_jag(
    r#","auth_type":"oauth2_id_jag","token_exchange_endpoint":"https://idp.example.com/org/token","id_jag_resource_token_endpoint":"https://upstream.example.com/token","client_id":"client-1","client_secret":"secret-1""#,
    Some(AuthSpecKind::IdJag)
)]
#[case::true_passthrough(r#","auth_type":"true_passthrough""#, Some(AuthSpecKind::Passthrough))]
#[case::oauth_delegate(r#","auth_type":"oauth_delegate""#, Some(AuthSpecKind::Passthrough))]
#[case::oauth2_token_exchange(
    r#","auth_type":"oauth2_token_exchange""#,
    Some(AuthSpecKind::TokenExchange)
)]
#[case::aws_sigv4(r#","auth_type":"aws_sigv4""#, None)]
#[case::none_oauth_passthrough(
    r#","auth_type":"none","oauth_passthrough":true,"extra_headers":["authorization"]"#,
    None
)]
#[case::none_without_the_authorization_header(
    r#","auth_type":"none","oauth_passthrough":true"#,
    Some(AuthSpecKind::None)
)]
#[case::byok_api_key(
    r#","auth_type":"api_key","is_byok":true,"authentication_token":"shared""#,
    None
)]
#[case::byok_token_exchange(
    r#","auth_type":"oauth2_token_exchange","is_byok":true"#,
    Some(AuthSpecKind::TokenExchange)
)]
fn each_auth_type_dispatch(#[case] extra: &str, #[case] expected: Option<AuthSpecKind>) {
    let resolved = mapped(extra);
    assert_eq!(resolved.map(|spec| spec.config.kind()), expected);
}

#[rstest]
#[case::api_key("api_key")]
#[case::bearer_token("bearer_token")]
#[case::token("token")]
#[case::authorization("authorization")]
#[case::basic("basic")]
fn static_modes_deferral_on_missing_token(#[case] auth_type: &str) {
    let absent = format!(r#","auth_type":"{auth_type}""#);
    assert!(mapped(&absent).is_none());
    let empty = format!(r#","auth_type":"{auth_type}","authentication_token":"""#);
    assert!(mapped(&empty).is_none());
    let shared = format!(r#","auth_type":"{auth_type}","authentication_token":"shared""#);
    let present = mapped(&shared).expect("a shared token keeps the mode owned");
    assert_eq!(present.config.kind(), AuthSpecKind::ApiKey);
}

#[rstest]
#[case::api_key(("api_key", "shared", "shared", "X-API-Key", "shared"))]
#[case::bearer_token(("bearer_token", "abc", "abc", "Authorization", "Bearer abc"))]
#[case::bearer_token_with_scheme(("bearer_token", "Bearer abc", "abc", "Authorization", "Bearer abc"))]
#[case::token(("token", "token abc", "abc", "Authorization", "token abc"))]
#[case::authorization(("authorization", "Bearer abc", "Bearer abc", "Authorization", "Bearer abc"))]
#[case::basic(("basic", "user:pass", "dXNlcjpwYXNz", "Authorization", "Basic dXNlcjpwYXNz"))]
#[case::basic_encoded(("basic", "Basic dXNlcjpwYXNz", "dXNlcjpwYXNz", "Authorization", "Basic dXNlcjpwYXNz"))]
fn static_modes_carry_the_shared_token(#[case] case: (&str, &str, &str, &str, &str)) {
    let (auth_type, token, expected_value, expected_name, expected_header) = case;
    let extra = format!(r#","auth_type":"{auth_type}","authentication_token":"{token}""#);
    let spec = owned(&extra);
    let config = api_key(&spec);
    assert_eq!(shared_value(config), expected_value);
    assert_eq!(
        config.header.header(expected_value),
        (expected_name.to_owned(), expected_header.to_owned())
    );
}

#[test]
fn static_modes_honour_the_upstream_token_header() {
    let spec = owned(
        r#","auth_type":"bearer_token","authentication_token":"abc","upstream_token_header":"X-Custom""#,
    );
    let config = api_key(&spec);
    assert_eq!(config.header.header("abc").0, "X-Custom");
}

#[test]
fn the_server_resource_is_the_url_or_the_server_id() {
    let with_url = owned(r#","auth_type":"api_key","authentication_token":"key""#);
    assert_eq!(with_url.resource, "https://mcp.example.com/mcp");
    assert_eq!(with_url.server_id, "srv-1");
    let encoded = r#"{"server_id":"srv-9","name":"Bare","transport":"http","url":"","auth_type":"api_key","authentication_token":"key"}"#;
    let bare = to_server_spec(&server_from(encoded))
        .expect("adapter mapping")
        .expect("owned");
    assert_eq!(bare.resource, "srv-9");
}

#[test]
fn client_credentials_deferral_on_missing_field() {
    // Pinned LiteLLM keeps client_credentials owned with grant fields missing and fails closed at resolve time; the DoD name predates that rule.
    let spec =
        owned(r#","auth_type":"oauth2","oauth2_flow":"client_credentials","client_id":"client-1""#);
    let config = client_credentials(&spec);
    assert_eq!(config.client_id.as_deref(), Some("client-1"));
    assert!(config.client_secret.is_none());
    assert!(config.token_url.is_none());
    assert!(config.upstream_resource.is_none());
}

#[test]
fn client_credentials_carries_the_grant_fields() {
    let extra = r#","auth_type":"oauth2","oauth2_flow":"client_credentials","client_id":"client-1","client_secret":"secret-1","token_url":"","configured_token_url":"https://idp.example.com/token","audience":"api://aud","scopes":["read","write"],"upstream_resource":"auto","token_endpoint_auth_method":"client_secret_post""#;
    let spec = owned(extra);
    let config = client_credentials(&spec);
    assert_eq!(config.client_id.as_deref(), Some("client-1"));
    assert_eq!(
        config.client_secret.as_ref().map(exposed),
        Some("secret-1".to_owned())
    );
    assert_eq!(
        config.token_url.as_deref(),
        Some("https://idp.example.com/token")
    );
    assert_eq!(config.scopes, vec!["read".to_owned(), "write".to_owned()]);
    assert_eq!(config.audience.as_deref(), Some("api://aud"));
    assert_eq!(
        config.upstream_resource.as_deref(),
        Some("https://mcp.example.com/mcp")
    );
    assert_eq!(
        config.token_endpoint_auth_method,
        Some(TokenEndpointAuthMethod::ClientSecretPost)
    );
}

#[test]
fn authorization_code_carries_the_upstream_token_header() {
    let spec = owned(r#","auth_type":"oauth2","upstream_token_header":"X-Custom""#);
    let AuthConfig::AuthorizationCode(config) = &spec.config else {
        panic!("expected authorization_code, got {}", spec.config.kind());
    };
    assert_eq!(config.header.header("token").0, "X-Custom");
    assert_eq!(config.header.header("token").1, "Bearer token");
    assert!(config.scopes.is_empty());
    assert!(config.client_id.is_none());
}

#[test]
fn token_exchange_deferred() {
    // Pinned LiteLLM keeps a declared oauth2_token_exchange owned even when its client configuration is incomplete; the DoD name predates that rule.
    let spec = owned(r#","auth_type":"oauth2_token_exchange""#);
    let config = token_exchange(&spec);
    assert_eq!(config.profile, TokenExchangeProfile::Rfc8693);
    assert_eq!(config.subject_token_type, DEFAULT_SUBJECT_TOKEN_TYPE);
    assert!(config.token_exchange_endpoint.is_none());
    assert!(config.client_id.is_none());
}

#[test]
fn token_exchange_carries_the_obo_settings() {
    let extra = r#","auth_type":"oauth2_token_exchange","token_exchange_endpoint":"","configured_token_url":"https://idp.example.com/token","token_exchange_profile":"entra_obo","subject_token_type":"urn:custom","client_id":"client-1","client_secret":"secret-1","audience":"api://aud","scopes":["a","b"],"upstream_token_header":"X-Custom""#;
    let spec = owned(extra);
    let config = token_exchange(&spec);
    assert_eq!(
        config.token_exchange_endpoint.as_deref(),
        Some("https://idp.example.com/token")
    );
    assert_eq!(config.profile, TokenExchangeProfile::EntraObo);
    assert_eq!(config.subject_token_type, "urn:custom");
    assert_eq!(config.client_id.as_deref(), Some("client-1"));
    assert_eq!(
        config.client_secret.as_ref().map(exposed),
        Some("secret-1".to_owned())
    );
    assert_eq!(config.audience.as_deref(), Some("api://aud"));
    assert_eq!(config.scopes, vec!["a".to_owned(), "b".to_owned()]);
    assert_eq!(config.header.header("token").0, "X-Custom");
}

#[test]
fn token_exchange_prefers_the_declared_endpoint() {
    let extra = r#","auth_type":"oauth2_token_exchange","token_exchange_endpoint":"https://declared.example.com/token","token_url":"https://fallback.example.com/token","token_exchange_profile":"other""#;
    let spec = owned(extra);
    let config = token_exchange(&spec);
    assert_eq!(
        config.token_exchange_endpoint.as_deref(),
        Some("https://declared.example.com/token")
    );
    assert_eq!(config.profile, TokenExchangeProfile::Rfc8693);
}

#[test]
fn id_jag_dispatch() {
    let extra = r#","client_secret":"secret-1","audience":"api://aud","id_jag_resource":"https://resource.example.com","scopes":["read","write"]"#;
    let config = id_jag_config(extra);
    assert_eq!(
        config.org_token_endpoint,
        "https://idp.example.com/org/token"
    );
    assert_eq!(
        config.resource_token_endpoint,
        "https://upstream.example.com/token"
    );
    assert_eq!(config.client_id, "client-1");
    assert_eq!(config.audience.as_deref(), Some("api://aud"));
    assert_eq!(
        config.resource.as_deref(),
        Some("https://resource.example.com")
    );
    assert_eq!(config.scopes, vec!["read".to_owned(), "write".to_owned()]);
    assert_eq!(config.header.header("token").0, "Authorization");
}

#[rstest]
#[case::no_org_endpoint(r#","id_jag_resource_token_endpoint":"https://upstream.example.com/token","client_id":"client-1","client_secret":"secret-1""#)]
#[case::empty_org_endpoint(r#","token_exchange_endpoint":"","id_jag_resource_token_endpoint":"https://upstream.example.com/token","client_id":"client-1","client_secret":"secret-1""#)]
#[case::no_resource_endpoint(r#","token_exchange_endpoint":"https://idp.example.com/org/token","client_id":"client-1","client_secret":"secret-1""#)]
#[case::no_client_id(r#","token_exchange_endpoint":"https://idp.example.com/org/token","id_jag_resource_token_endpoint":"https://upstream.example.com/token","client_secret":"secret-1""#)]
#[case::empty_client_id(r#","token_exchange_endpoint":"https://idp.example.com/org/token","id_jag_resource_token_endpoint":"https://upstream.example.com/token","client_id":"","client_secret":"secret-1""#)]
#[case::no_client_auth(r#","token_exchange_endpoint":"https://idp.example.com/org/token","id_jag_resource_token_endpoint":"https://upstream.example.com/token","client_id":"client-1""#)]
#[case::empty_client_secret(r#","token_exchange_endpoint":"https://idp.example.com/org/token","id_jag_resource_token_endpoint":"https://upstream.example.com/token","client_id":"client-1","client_secret":"""#)]
fn id_jag_deferral_on_half_configured(#[case] extra: &str) {
    let encoded = format!(
        r#"{{"server_id":"srv-1","name":"Server One","transport":"http","url":"https://mcp.example.com/mcp","auth_type":"oauth2_id_jag"{extra}}}"#
    );
    let record = server_from(&encoded);
    let resolved = to_server_spec(&record).expect("adapter mapping");
    assert!(resolved.is_none());
}

#[rstest]
#[case::private_key_and_secret(
    r#","client_private_key":"pk","client_secret":"secret-1","client_private_key_id":"kid","client_assertion_signing_alg":"ES256""#,
    "pk",
    Some("kid"),
    "ES256"
)]
#[case::private_key_only(r#","client_private_key":"pk""#, "pk", None, "RS256")]
#[case::secret_only(r#","client_secret":"secret-1""#, "secret-1", None, "")]
#[case::empty_private_key_falls_to_secret(
    r#","client_private_key":"","client_secret":"secret-1""#,
    "secret-1",
    None,
    ""
)]
fn id_jag_client_auth_selection(
    #[case] extra: &str,
    #[case] expected_key: &str,
    #[case] expected_key_id: Option<&str>,
    #[case] expected_alg: &str,
) {
    let config = id_jag_config(extra);
    match config.client_auth {
        ClientAuth::PrivateKeyJwt(jwt) => {
            assert_eq!(exposed(&jwt.private_key), expected_key);
            assert_eq!(jwt.key_id.as_deref(), expected_key_id);
            assert_eq!(jwt.signing_alg, expected_alg);
        }
        ClientAuth::ClientSecret(secret_auth) => {
            assert_eq!(exposed(&secret_auth.client_secret), expected_key);
            assert!(expected_key_id.is_none());
        }
    }
}

#[rstest]
#[case::default(r#","client_secret":"secret-1""#)]
#[case::empty(r#","client_secret":"secret-1","subject_token_type":"""#)]
#[case::access_token(r#","client_secret":"secret-1","subject_token_type":"urn:ietf:params:oauth:token-type:access_token""#)]
fn id_jag_subject_token_type_default_mapping(#[case] extra: &str) {
    let config = id_jag_config(extra);
    assert_eq!(config.subject_token_type, ID_JAG_SUBJECT_TOKEN_TYPE);
    assert_ne!(config.subject_token_type, DEFAULT_SUBJECT_TOKEN_TYPE);
}

#[rstest]
#[case::saml2(
    r#","client_secret":"secret-1","subject_token_type":"urn:ietf:params:oauth:token-type:saml2""#,
    "urn:ietf:params:oauth:token-type:saml2"
)]
#[case::custom(
    r#","client_secret":"secret-1","subject_token_type":"custom-assertion""#,
    "custom-assertion"
)]
fn id_jag_subject_token_type_keeps_an_explicit_value(#[case] extra: &str, #[case] expected: &str) {
    let config = id_jag_config(extra);
    assert_eq!(config.subject_token_type, expected);
}
