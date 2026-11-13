#![cfg(test)]

use betterlite_mcp::credential_contracts::auth_config::oauth::ClientCredentialsConfig;
use betterlite_mcp::credential_contracts::errors::CredError;
use betterlite_mcp::credential_contracts::header_carrier::HeaderCarrier;
use betterlite_mcp::credential_contracts::token_endpoint_auth::TokenEndpointAuthMethod;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{parse_expires_in, parse_granted_scopes, prepare_grant};
use crate::mcp::outbound_credentials::client_credentials::token_post::TokenResponseBody;

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn config(client_id: &str, client_secret: &str) -> ClientCredentialsConfig {
    ClientCredentialsConfig {
        header: HeaderCarrier::default(),
        client_id: Some(client_id.to_owned()),
        client_secret: Some(secret(client_secret)),
        token_url: Some("https://idp.example.com/token".to_owned()),
        scopes: vec!["read".to_owned()],
        audience: None,
        upstream_resource: None,
        token_endpoint_auth_method: None,
    }
}

fn empty_config() -> ClientCredentialsConfig {
    ClientCredentialsConfig {
        header: HeaderCarrier::default(),
        client_id: None,
        client_secret: None,
        token_url: None,
        scopes: Vec::new(),
        audience: None,
        upstream_resource: None,
        token_endpoint_auth_method: None,
    }
}

fn pairs(fields: &[(String, SecretString)]) -> Vec<(String, String)> {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

fn pair(name: &str, value: &str) -> (String, String) {
    (name.to_owned(), value.to_owned())
}

fn expires_in(raw: &str) -> Option<i64> {
    let body: TokenResponseBody =
        serde_json::from_str(&format!("{{\"expires_in\": {raw}}}")).unwrap();
    parse_expires_in(body.expires_in.as_ref())
}

fn granted_scopes(raw: &str) -> Vec<String> {
    let body: TokenResponseBody = serde_json::from_str(&format!("{{\"scope\": {raw}}}")).unwrap();
    parse_granted_scopes(body.scope.as_ref())
}

#[test]
fn prepare_grant_orders_the_form_the_way_litellm_builds_it() {
    let mut settings = config("client", "secret");
    settings.scopes = vec!["read".to_owned(), "write".to_owned()];
    settings.audience = Some("api://aud".to_owned());
    settings.upstream_resource = Some("https://api.example.com".to_owned());
    settings.token_endpoint_auth_method = Some(TokenEndpointAuthMethod::ClientSecretPost);
    let grant = prepare_grant(&settings).unwrap();
    assert_eq!(
        pairs(&grant.form),
        vec![
            pair("grant_type", "client_credentials"),
            pair("client_id", "client"),
            pair("client_secret", "secret"),
            pair("scope", "read write"),
            pair("audience", "api://aud"),
            pair("resource", "https://api.example.com"),
        ]
    );
    assert_eq!(grant.token_url, "https://idp.example.com/token");
    assert!(grant.headers.is_empty());
}

#[test]
fn prepare_grant_leaves_empty_optional_values_out_of_the_form() {
    let mut settings = config("client", "secret");
    settings.scopes = Vec::new();
    settings.audience = Some(String::new());
    settings.upstream_resource = Some(String::new());
    let grant = prepare_grant(&settings).unwrap();
    assert_eq!(
        pairs(&grant.form),
        vec![
            pair("grant_type", "client_credentials"),
            pair("client_id", "client"),
            pair("client_secret", "secret"),
        ]
    );
}

#[test]
fn prepare_grant_basic_puts_the_credentials_in_the_authorization_header() {
    let mut settings = config("client", "secret");
    settings.token_endpoint_auth_method = Some(TokenEndpointAuthMethod::ClientSecretBasic);
    let grant = prepare_grant(&settings).unwrap();
    assert_eq!(
        pairs(&grant.form),
        vec![
            pair("grant_type", "client_credentials"),
            pair("scope", "read"),
        ]
    );
    assert_eq!(
        pairs(&grant.headers),
        vec![pair("Authorization", "Basic Y2xpZW50OnNlY3JldA==")]
    );
}

#[rstest]
#[case::everything(
    None,
    None,
    None,
    "client_credentials config is missing: client_id, client_secret, token_url"
)]
#[case::only_id(
    Some(""),
    Some("s"),
    Some("u"),
    "client_credentials config is missing: client_id"
)]
#[case::only_secret(
    Some("i"),
    Some(""),
    Some("u"),
    "client_credentials config is missing: client_secret"
)]
#[case::only_url(
    Some("i"),
    Some("s"),
    Some(""),
    "client_credentials config is missing: token_url"
)]
#[case::client_and_url(
    None,
    Some("s"),
    Some(""),
    "client_credentials config is missing: client_id, token_url"
)]
fn prepare_grant_names_every_missing_field_in_order(
    #[case] client_id: Option<&str>,
    #[case] client_secret: Option<&str>,
    #[case] token_url: Option<&str>,
    #[case] expected: &str,
) {
    let mut settings = empty_config();
    settings.client_id = client_id.map(str::to_owned);
    settings.client_secret = client_secret.map(secret);
    settings.token_url = token_url.map(str::to_owned);
    let failure = prepare_grant(&settings).unwrap_err();
    assert_eq!(failure, CredError::Misconfigured(expected.to_owned()));
}

fn rotate_secret(settings: &mut ClientCredentialsConfig) {
    settings.client_secret = Some(secret("rotated"));
}

fn rescope(settings: &mut ClientCredentialsConfig) {
    settings.scopes = vec!["read".to_owned(), "admin".to_owned()];
}

fn drop_scopes(settings: &mut ClientCredentialsConfig) {
    settings.scopes = Vec::new();
}

fn set_audience(settings: &mut ClientCredentialsConfig) {
    settings.audience = Some("api://aud".to_owned());
}

fn set_resource(settings: &mut ClientCredentialsConfig) {
    settings.upstream_resource = Some("https://api.example.com".to_owned());
}

fn use_basic(settings: &mut ClientCredentialsConfig) {
    settings.token_endpoint_auth_method = Some(TokenEndpointAuthMethod::ClientSecretBasic);
}

fn move_url(settings: &mut ClientCredentialsConfig) {
    settings.token_url = Some("https://idp.example.com/other".to_owned());
}

fn rename_client(settings: &mut ClientCredentialsConfig) {
    settings.client_id = Some("other-client".to_owned());
}

#[test]
fn identity_key_hashes_the_seven_values_with_newlines() {
    let mut settings = config("client", "secret");
    settings.scopes = vec!["read".to_owned(), "write".to_owned()];
    settings.audience = Some("api://aud".to_owned());
    settings.upstream_resource = Some("https://api.example.com".to_owned());
    settings.token_endpoint_auth_method = Some(TokenEndpointAuthMethod::ClientSecretPost);
    let grant = prepare_grant(&settings).unwrap();
    assert_eq!(
        grant.identity_key,
        "d98266a352f0d1c16a1a60a33551fc21fbdd31d6caa8c3251110d271863ef707"
    );
}

#[rstest]
#[case::secret(rotate_secret)]
#[case::scopes(rescope)]
#[case::no_scopes(drop_scopes)]
#[case::audience(set_audience)]
#[case::resource(set_resource)]
#[case::auth_method(use_basic)]
#[case::token_url(move_url)]
#[case::client_id(rename_client)]
fn identity_key_changes_with_every_value_it_names(
    #[case] mutate: fn(&mut ClientCredentialsConfig),
) {
    let base = config("client", "secret");
    let mut changed = config("client", "secret");
    mutate(&mut changed);
    let base_key = prepare_grant(&base).unwrap().identity_key;
    let changed_key = prepare_grant(&changed).unwrap().identity_key;
    assert_ne!(base_key, changed_key);
}

#[test]
fn identity_key_is_stable_for_the_same_config() {
    let first = prepare_grant(&config("client", "secret")).unwrap();
    let second = prepare_grant(&config("client", "secret")).unwrap();
    assert_eq!(first.identity_key, second.identity_key);
    assert_ne!(first.identity_key.len(), 0);
}

#[rstest]
#[case::integer("3600", Some(3600))]
#[case::zero("0", Some(0))]
#[case::negative("-5", Some(-5))]
#[case::text(" \"3600\"", Some(3600))]
#[case::padded_text(" \" 3600 \"", Some(3600))]
#[case::signed_text(" \"+3600\"", Some(3600))]
#[case::underscored_text(" \"3_600\"", Some(3600))]
#[case::float("3600.5", None)]
#[case::float_whole("3600.0", None)]
#[case::truthy("true", None)]
#[case::falsy("false", None)]
#[case::null("null", None)]
#[case::array("[]", None)]
#[case::object("{}", None)]
#[case::words(" \"soon\"", None)]
#[case::double_underscore(" \"3__600\"", None)]
#[case::trailing_underscore(" \"3600_\"", None)]
#[case::leading_underscore(" \"_3600\"", None)]
#[case::hex(" \"0x10\"", None)]
#[case::empty_text(" \"\"", None)]
#[case::text_too_large(" \"9223372036854775808\"", None)]
fn parse_expires_in_follows_python_int(#[case] raw: &str, #[case] expected: Option<i64>) {
    assert_eq!(expires_in(raw), expected);
}

#[test]
fn parse_expires_in_accepts_the_largest_signed_value() {
    assert_eq!(expires_in(" \"9223372036854775807\""), Some(i64::MAX));
    assert_eq!(expires_in(" \"-9223372036854775808\""), Some(i64::MIN));
}

#[rstest]
#[case::words(" \"read write\"", vec!["read".to_owned(), "write".to_owned()])]
#[case::single(" \"read\"", vec!["read".to_owned()])]
#[case::spaces(" \"  read \\twrite \"", vec!["read".to_owned(), "write".to_owned()])]
#[case::blank(" \"   \"", vec![])]
#[case::empty(" \"\"", vec![])]
#[case::number("7", vec![])]
#[case::truthy("true", vec![])]
#[case::null("null", vec![])]
#[case::array("[\"read\"]", vec![])]
fn parse_granted_scopes_splits_only_a_non_empty_string(
    #[case] raw: &str,
    #[case] expected: Vec<String>,
) {
    assert_eq!(granted_scopes(raw), expected);
}
