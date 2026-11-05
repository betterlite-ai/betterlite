#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::decode_payload;
use crate::google_secret_manager::GoogleSecretManagerError;

#[rstest]
#[case::plain("c2VjcmV0", "secret")]
#[case::padded("c2VjcmV0cw==", "secrets")]
#[case::multibyte("w6Nnw7w=", "\u{e3}g\u{fc}")]
#[case::numeric_text("NDU2", "456")]
#[case::empty_payload("", "")]
fn decodes_base64_payload_data(#[case] encoded: &str, #[case] expected: &str) {
    let body = format!(r#"{{"name":"n","payload":{{"data":"{encoded}","dataCrc32c":"1"}}}}"#);

    let secret = decode_payload(&body).unwrap().unwrap();

    assert_eq!(secret.expose_secret(), expected);
}

#[rstest]
#[case::empty_object("{}")]
#[case::empty_payload(r#"{"payload":{}}"#)]
#[case::null_data(r#"{"payload":{"data":null}}"#)]
#[case::null_payload(r#"{"payload":null}"#)]
fn missing_payload_data_is_absent(#[case] body: &str) {
    assert!(decode_payload(body).unwrap().is_none());
}

#[test]
fn malformed_base64_is_an_error() {
    let error = decode_payload(r#"{"payload":{"data":"c2Vj*"}}"#).unwrap_err();

    assert!(matches!(error, GoogleSecretManagerError::Base64(_)));
}

#[test]
fn missing_padding_is_an_error() {
    let error = decode_payload(r#"{"payload":{"data":"c2VjcmV0cw"}}"#).unwrap_err();

    assert!(matches!(error, GoogleSecretManagerError::Base64(_)));
}

#[test]
fn invalid_utf8_is_an_error() {
    let error = decode_payload(r#"{"payload":{"data":"/w=="}}"#).unwrap_err();

    assert!(matches!(error, GoogleSecretManagerError::Utf8(_)));
}

#[rstest]
#[case::not_json("not json")]
#[case::array("[]")]
#[case::number_data(r#"{"payload":{"data":5}}"#)]
fn unparsable_body_is_an_error(#[case] body: &str) {
    let error = decode_payload(body).unwrap_err();

    assert!(matches!(error, GoogleSecretManagerError::Response(_)));
}
