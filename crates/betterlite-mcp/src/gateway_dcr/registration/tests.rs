#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use chrono::{DateTime, Utc};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use crate::gateway_dcr::contracts::{
    GATEWAY_DCR_CLIENT_ID_PREFIX, MAX_CLIENT_ID_LENGTH, TOKEN_EXCHANGE_GRANT_TYPE,
};
use crate::gateway_dcr::models::GatewayDcrClient;
use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};
use crate::gateway_dcr::registration::{RegisteredClient, Registration, register_aggregate_client};
use crate::gateway_dcr::sealed::open_gateway_dcr_client;
use crate::redirect_trust::shape::native_callbacks;

const LIST_MESSAGE: &str = "redirect_uris must be a list of 1 to 4 URIs";
const ITEM_MESSAGE: &str = "each redirect URI must be a string of at most 256 characters";
const SCHEME_MESSAGE: &str =
    "each redirect URI must be https, http on a loopback host, or a registered native callback";

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-01-02T03:04:05+00:00")
        .unwrap()
        .with_timezone(&Utc)
}

fn cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("b4674-registration-salt")),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn cipher_without_a_key() -> DbCipher {
    let mut cipher = cipher();
    cipher.settings.signing_key = None;
    cipher
}

fn uri(text: &str) -> JsonValue {
    JsonValue::String(text.to_owned())
}

fn body(redirect_uris: Vec<JsonValue>) -> BTreeMap<String, JsonValue> {
    BTreeMap::from([("redirect_uris".to_owned(), JsonValue::Array(redirect_uris))])
}

fn register(body: &BTreeMap<String, JsonValue>) -> Registration {
    register_aggregate_client(body, false, now(), &cipher(), &native_callbacks(""))
}

fn register_with(cipher: &DbCipher, redirect_uris: Vec<JsonValue>) -> Registration {
    register_aggregate_client(
        &body(redirect_uris),
        false,
        now(),
        cipher,
        &native_callbacks(""),
    )
}

fn boundary_uris(third: usize, fourth: usize) -> Vec<JsonValue> {
    vec![
        uri(&uri_of_characters(256, '\u{10348}')),
        uri(&uri_of_characters(256, 'a')),
        uri(&uri_of_characters(third, 'a')),
        uri(&uri_of_characters(fourth, 'a')),
    ]
}

fn registered(registration: Registration) -> RegisteredClient {
    match registration {
        Registration::Registered(client) => client,
        Registration::Refused(error) => {
            panic!("expected a registration, got {}", error.description)
        }
    }
}

fn refusal_of(registration: Registration) -> OauthError {
    match registration {
        Registration::Refused(error) => error,
        Registration::Registered(client) => panic!("expected a refusal, got {}", client.client_id),
    }
}

fn message(redirect_uris: Vec<JsonValue>) -> String {
    refusal_of(register(&body(redirect_uris))).description
}

fn uri_of_characters(total: usize, symbol: char) -> String {
    let prefix = "https://a.example/";
    let fill = total.saturating_sub(prefix.chars().count());
    format!("{prefix}{}", symbol.to_string().repeat(fill))
}

#[test]
fn rejects_a_missing_or_null_redirect_uris() {
    let missing = refusal_of(register(&BTreeMap::new()));
    assert_eq!(missing.status, 400);
    assert_eq!(missing.error, OauthErrorCode::InvalidRedirectUri);
    assert_eq!(missing.description, LIST_MESSAGE);

    let mut request = body(Vec::new());
    request.insert("redirect_uris".to_owned(), JsonValue::Null);
    assert_eq!(refusal_of(register(&request)).description, LIST_MESSAGE);
}

#[rstest]
#[case::string(JsonValue::String("https://client.example/callback".to_owned()))]
#[case::bool(JsonValue::Bool(true))]
#[case::number(JsonValue::Number(Number::from(1_i32)))]
#[case::object(JsonValue::Object(BTreeMap::new()))]
fn rejects_a_redirect_uris_that_is_not_an_array(#[case] value: JsonValue) {
    let mut request = body(Vec::new());
    request.insert("redirect_uris".to_owned(), value);
    assert_eq!(refusal_of(register(&request)).description, LIST_MESSAGE);
}

#[test]
fn rejects_an_empty_or_over_four_redirect_uris() {
    assert_eq!(message(Vec::new()), LIST_MESSAGE);
    let five = vec![
        uri("https://a.example/1"),
        uri("https://a.example/2"),
        uri("https://a.example/3"),
        uri("https://a.example/4"),
        uri("https://a.example/5"),
    ];
    assert_eq!(message(five), LIST_MESSAGE);
}

#[test]
fn rejects_a_non_string_item_before_looking_at_any_uri() {
    let error = message(vec![
        uri("http://evil.example/callback"),
        JsonValue::Bool(false),
    ]);
    assert_eq!(error, ITEM_MESSAGE);
    assert_eq!(
        message(vec![uri("https://ok.example/callback"), JsonValue::Null]),
        ITEM_MESSAGE
    );
}

#[test]
fn counts_the_uri_length_in_characters_not_bytes() {
    let at_the_limit = uri_of_characters(256, '\u{10348}');
    assert_eq!(at_the_limit.chars().count(), 256);
    assert!(at_the_limit.len() > 256);
    let accepted = registered(register(&body(vec![uri(&at_the_limit)])));
    assert_eq!(accepted.redirect_uris, vec![at_the_limit]);

    let over_the_limit = uri_of_characters(257, 'a');
    assert_eq!(message(vec![uri(&over_the_limit)]), ITEM_MESSAGE);
}

#[rstest]
#[case::https("https://any-host.example/callback")]
#[case::https_with_a_query("https://any-host.example/callback?client=1")]
#[case::localhost("http://localhost/callback")]
#[case::localhost_with_a_port("http://localhost:8080/callback")]
#[case::loopback_ipv4("http://127.0.0.1/callback")]
#[case::loopback_ipv6("http://[::1]:9000/callback")]
#[case::native_callback("cursor://anysphere.cursor-mcp/oauth/callback")]
fn accepts_https_and_loopback_http_and_a_native_callback(#[case] accepted: &str) {
    let client = registered(register(&body(vec![uri(accepted)])));
    assert_eq!(client.redirect_uris, vec![accepted.to_owned()]);
}

#[rstest]
#[case::plain_http("http://evil.example/callback")]
#[case::localhost_subdomain("http://localhost.evil.example/callback")]
#[case::loopback_subdomain("http://127.0.0.1.evil.example/callback")]
fn rejects_plain_http_on_a_non_loopback_host(#[case] rejected: &str) {
    assert_eq!(message(vec![uri(rejected)]), SCHEME_MESSAGE);
}

#[test]
fn rejects_a_fragment_with_the_shared_validator_detail() {
    assert_eq!(
        message(vec![uri("https://client.example/callback#fragment")]),
        "redirect_uri must not contain a URL fragment (#...)."
    );
}

#[test]
fn rejects_userinfo_with_the_shared_validator_detail() {
    assert_eq!(
        message(vec![uri("https://claude.ai@attacker.example/callback")]),
        "redirect_uri must not contain userinfo (user:pass@host)."
    );
}

#[test]
fn rejects_a_backslash_host_with_the_shared_validator_detail() {
    assert_eq!(
        message(vec![uri("https://good.example\\evil.example/callback")]),
        "redirect_uri host must not contain backslashes."
    );
}

#[test]
fn rejects_an_unparsable_uri_as_a_client_error() {
    let error = refusal_of(register(&body(vec![uri("https://[::1/callback")])));
    assert_eq!(error.status, 400);
    assert_eq!(error.error, OauthErrorCode::InvalidRedirectUri);
    assert_eq!(error.description, "each redirect URI must be a valid URL");
}

#[test]
fn registers_a_public_client_and_drops_the_request_metadata() {
    let mut request = body(vec![uri("https://client.example/callback")]);
    request.insert(
        "client_name".to_owned(),
        JsonValue::String("Client".to_owned()),
    );
    request.insert(
        "token_endpoint_auth_method".to_owned(),
        JsonValue::String("client_secret_basic".to_owned()),
    );
    request.insert(
        "grant_types".to_owned(),
        JsonValue::Array(vec![uri("implicit")]),
    );
    request.insert(
        "response_types".to_owned(),
        JsonValue::Array(vec![uri("token")]),
    );
    let client = registered(register(&request));
    assert!(client.client_id.starts_with(GATEWAY_DCR_CLIENT_ID_PREFIX));
    assert_eq!(client.token_endpoint_auth_method, "none");
    assert_eq!(
        client.grant_types,
        vec!["authorization_code", "refresh_token"]
    );
    assert_eq!(client.response_types, ["code"]);
    let text = serde_json::to_string(&client).unwrap();
    assert!(!text.contains("client_name"));
    assert!(!text.contains("client_secret_basic"));
}

#[test]
fn a_registration_answers_the_public_client_body_exactly() {
    let registration = register(&body(vec![uri("https://client.example/callback")]));
    assert_eq!(registration.status(), 201);
    let client = registered(registration);
    assert_eq!(client.client_id_issued_at, now().timestamp());
    assert_eq!(
        client.redirect_uris,
        vec!["https://client.example/callback".to_owned()]
    );
    assert_eq!(
        serde_json::to_string(&client).unwrap(),
        format!(
            "{{\"client_id\":\"{}\",\"client_id_issued_at\":{},\"redirect_uris\":[\"https://client.example/callback\"],\"token_endpoint_auth_method\":\"none\",\"grant_types\":[\"authorization_code\",\"refresh_token\"],\"response_types\":[\"code\"]}}",
            client.client_id,
            now().timestamp()
        )
    );
}

#[test]
fn appends_the_token_exchange_grant_when_the_flow_offers_it() {
    let registration = register_aggregate_client(
        &body(vec![uri("https://client.example/callback")]),
        true,
        now(),
        &cipher(),
        &native_callbacks(""),
    );
    let client = registered(registration);
    assert_eq!(
        client.grant_types,
        vec![
            "authorization_code",
            "refresh_token",
            TOKEN_EXCHANGE_GRANT_TYPE
        ]
    );
    assert!(
        serde_json::to_string(&client)
            .unwrap()
            .contains("urn:ietf:params:oauth:grant-type:token-exchange")
    );
}

#[test]
fn registers_the_issued_client_id_so_it_opens_to_the_sent_uris() {
    let sent = vec![
        uri("https://client.example/callback"),
        uri("http://localhost:8080/callback"),
    ];
    let client = registered(register(&body(sent)));
    let opened = open_gateway_dcr_client(&client.client_id, &cipher()).unwrap();
    assert_eq!(
        opened,
        GatewayDcrClient {
            redirect_uris: vec![
                "https://client.example/callback".to_owned(),
                "http://localhost:8080/callback".to_owned(),
            ],
            iat: now().timestamp(),
        }
    );
}

#[test]
fn rejects_a_sealed_client_id_over_the_length_cap() {
    let longest = uri_of_characters(256, '\u{10348}');
    let error = refusal_of(register(&body(vec![
        uri(&longest),
        uri(&longest),
        uri(&longest),
        uri(&longest),
    ])));
    assert_eq!(error.status, 400);
    assert_eq!(error.error, OauthErrorCode::InvalidClientMetadata);
    assert_eq!(error.description, "registered metadata is too large");
}

#[test]
fn accepts_a_sealed_client_id_exactly_at_the_length_cap() {
    // An AES id is 16 + 4k characters, so only a 1494-1496 byte payload lands on the 2048 cap.
    let mut cipher = cipher();
    cipher.settings.algorithm = Algorithm::Aes256Gcm;
    let at_the_cap = registered(register_with(&cipher, boundary_uris(110, 110)));
    assert_eq!(at_the_cap.client_id.chars().count(), MAX_CLIENT_ID_LENGTH);
    let opened = open_gateway_dcr_client(&at_the_cap.client_id, &cipher).unwrap();
    assert_eq!(opened.redirect_uris.len(), 4);

    let error = refusal_of(register_with(&cipher, boundary_uris(111, 112)));
    assert_eq!(error.status, 400);
    assert_eq!(error.error, OauthErrorCode::InvalidClientMetadata);
    assert_eq!(error.description, "registered metadata is too large");
}

#[test]
fn answers_500_when_the_cipher_refuses_to_seal() {
    let error = refusal_of(register_aggregate_client(
        &body(vec![uri("https://client.example/callback")]),
        false,
        now(),
        &cipher_without_a_key(),
        &native_callbacks(""),
    ));
    assert_eq!(error.status, 500);
    assert_eq!(error.error, OauthErrorCode::ServerError);
    assert_eq!(
        error.description,
        "the client registration could not be sealed"
    );
}

#[test]
fn checks_every_uri_in_order() {
    let error = refusal_of(register(&body(vec![
        uri("https://client.example/callback"),
        uri("http://evil.example/callback"),
    ])));
    assert_eq!(error.description, SCHEME_MESSAGE);
    assert_eq!(error.status, 400);
}
