#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::vault::error::VaultResponseError;
use crate::vault::response::{decode_login_response, json_object_body, secret_value};

const LOGIN: &str = r#"{"auth":{"client_token":"hvs.token","lease_duration":3600}}"#;

#[test]
fn a_login_response_yields_the_token_and_the_lease() {
    let login = decode_login_response(LOGIN.as_bytes()).unwrap();
    assert_eq!(login.client_token.expose_secret(), "hvs.token");
    assert_eq!(login.lease_duration, 3600);
}

#[rstest]
#[case::not_json("not a login response", VaultResponseError::InvalidJson)]
#[case::array_body("[]", VaultResponseError::MissingAuth)]
#[case::missing_auth("{}", VaultResponseError::MissingAuth)]
#[case::auth_not_an_object(r#"{"auth":"token"}"#, VaultResponseError::MissingAuth)]
#[case::missing_token(r#"{"auth":{"lease_duration":60}}"#, VaultResponseError::MissingToken)]
#[case::token_not_a_string(
    r#"{"auth":{"client_token":12,"lease_duration":60}}"#,
    VaultResponseError::MissingToken
)]
#[case::missing_lease(
    r#"{"auth":{"client_token":"hvs.token"}}"#,
    VaultResponseError::MissingLease
)]
#[case::negative_lease(
    r#"{"auth":{"client_token":"hvs.token","lease_duration":-1}}"#,
    VaultResponseError::MissingLease
)]
#[case::fractional_lease(
    r#"{"auth":{"client_token":"hvs.token","lease_duration":1.5}}"#,
    VaultResponseError::MissingLease
)]
fn a_broken_login_response_is_refused(#[case] body: &str, #[case] expected: VaultResponseError) {
    assert_eq!(
        decode_login_response(body.as_bytes()).unwrap_err(),
        expected
    );
}

#[test]
fn a_broken_login_error_never_quotes_the_body() {
    let body = r#"{"auth":{"client_token":"hvs.leak-token","lease_duration":"forever"}}"#;
    let error = decode_login_response(body.as_bytes()).unwrap_err();
    let text = format!("{error} {error:?}");
    assert!(!text.contains("hvs.leak-token"));
    assert!(!text.contains("forever"));
}

#[test]
fn a_json_object_body_is_returned_as_is() {
    let object = json_object_body(br#"{"data":{"version":1},"warnings":null}"#).unwrap();
    assert_eq!(object.len(), 2);
    assert!(
        object
            .get("data")
            .and_then(|value| value.as_object())
            .is_some()
    );
}

#[rstest]
#[case::not_json("not json", VaultResponseError::InvalidJson)]
#[case::array("[]", VaultResponseError::NotAnObject)]
#[case::string("\"text\"", VaultResponseError::NotAnObject)]
#[case::null("null", VaultResponseError::NotAnObject)]
fn a_non_object_body_is_refused(#[case] body: &str, #[case] expected: VaultResponseError) {
    assert_eq!(json_object_body(body.as_bytes()).unwrap_err(), expected);
}

#[test]
fn a_kv_v2_body_yields_the_secret_value() {
    let body =
        br#"{"data":{"data":{"key":"Vault Is The Way","password":"pw"},"metadata":{"version":1}}}"#;
    assert_eq!(
        secret_value(body, "key").map(|value| value.expose_secret().to_owned()),
        Some("Vault Is The Way".to_owned())
    );
    assert_eq!(
        secret_value(body, "password").map(|value| value.expose_secret().to_owned()),
        Some("pw".to_owned())
    );
}

#[rstest]
#[case::invalid_json(b"not json".as_slice())]
#[case::missing_data(b"{}".as_slice())]
#[case::data_not_an_object(br#"{"data":[]}"#.as_slice())]
#[case::missing_inner_data(br#"{"data":{"metadata":{}}}"#.as_slice())]
#[case::inner_data_not_an_object(br#"{"data":{"data":"value"}}"#.as_slice())]
#[case::missing_key(br#"{"data":{"data":{"other":"value"}}}"#.as_slice())]
#[case::non_string_value(br#"{"data":{"data":{"key":42}}}"#.as_slice())]
#[case::null_value(br#"{"data":{"data":{"key":null}}}"#.as_slice())]
fn a_missing_or_wrong_secret_value_is_none(#[case] body: &[u8]) {
    assert!(secret_value(body, "key").is_none());
}

#[test]
fn a_repeated_key_keeps_the_last_value() {
    let body = br#"{"data":{"data":{"key":"first","key":"second"}}}"#;
    assert_eq!(
        secret_value(body, "key").map(|value| value.expose_secret().to_owned()),
        Some("second".to_owned())
    );
}

#[test]
fn the_data_key_selects_one_field_of_the_secret_object() {
    let body = br#"{"data":{"data":{"key":"default","token":"selected","count":3}}}"#;
    assert_eq!(
        secret_value(body, "token").map(|value| value.expose_secret().to_owned()),
        Some("selected".to_owned())
    );
}

#[test]
fn replays_the_vault_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/vault_response");
    let decoded = fs::read_dir(&corpus)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            (
                decode_login_response(&bytes),
                json_object_body(&bytes),
                secret_value(&bytes, "key"),
            )
        })
        .collect::<Vec<_>>();
    assert!(decoded.len() >= 6);
}
