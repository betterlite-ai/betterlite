#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_db::resource_rows::ResourceRowsError;
use betterlite_db::resource_rows::credential::{CreateCredentialItem, UpdateCredentialItem};
use rstest::rstest;

use super::super::error::CredentialError;
use super::super::request::{MAX_CREDENTIAL_BODY_BYTES, decode_credential_request};
use super::support::{payload, text};

const NULL_BODY: &[u8] = b"null";
const ARRAY_BODY: &[u8] = b"[]";
const TEXT_BODY: &[u8] = b"\"text\"";

fn body_of_size(size: usize) -> Vec<u8> {
    let head = br#"{"credential_name":"x","credential_info":{},"credential_values":{"api_key":""#;
    let tail = br#""}}"#;
    let mut body = Vec::with_capacity(size);
    body.extend_from_slice(head);
    let padding = size
        .checked_sub(tail.len())
        .expect("the size leaves room for the tail");
    body.resize(padding, b'a');
    body.extend_from_slice(tail);
    assert_eq!(size, body.len());
    body
}

#[test]
fn decodes_a_create_request() {
    let request = decode_credential_request::<CreateCredentialItem>(
        br#"{"credential_name":"openai","credential_info":{"team":"ml"},"credential_values":{"api_key":"sk-1"}}"#,
    )
    .expect("the request decodes");
    assert_eq!("openai", request.fields.base.credential_name);
    assert_eq!(
        payload(&[("team", text("ml"))]),
        request.fields.base.credential_info
    );
    assert_eq!(
        Some(payload(&[("api_key", text("sk-1"))])),
        request.fields.credential_values
    );
}

#[test]
fn decodes_an_update_request_without_values() {
    let request = decode_credential_request::<UpdateCredentialItem>(
        br#"{"credential_name":"","credential_info":{},"credential_values":null}"#,
    )
    .expect("the request decodes");
    assert_eq!("", request.fields.credential_name);
    assert_eq!(None, request.fields.credential_values);
    assert_eq!(None, request.fields.model_id);
}

#[test]
fn a_create_request_without_values_or_model_is_refused() {
    let result = decode_credential_request::<CreateCredentialItem>(
        br#"{"credential_name":"openai","credential_info":{}}"#,
    );
    assert!(matches!(
        result,
        Err(CredentialError::Rows(
            ResourceRowsError::CredentialValueRequired
        ))
    ));
}

#[test]
fn an_update_request_without_credential_info_is_refused() {
    let result =
        decode_credential_request::<UpdateCredentialItem>(br#"{"credential_name":"openai"}"#);
    assert!(matches!(result, Err(CredentialError::Rows(_))));
}

#[rstest]
#[case::null(NULL_BODY)]
#[case::array(ARRAY_BODY)]
#[case::text(TEXT_BODY)]
fn a_non_object_body_is_refused(#[case] body: &[u8]) {
    drop(decode_credential_request::<CreateCredentialItem>(body).unwrap_err());
    drop(decode_credential_request::<UpdateCredentialItem>(body).unwrap_err());
}

#[test]
fn a_non_string_credential_value_is_refused_without_the_value() {
    let result = decode_credential_request::<CreateCredentialItem>(
        br#"{"credential_name":"openai","credential_info":{},"credential_values":"sk-secret-leak"}"#,
    );
    let Err(error) = result else {
        panic!("the request is refused");
    };
    let message = error.to_string();
    assert!(message.contains("credential_values"), "{message}");
    assert!(!message.contains("sk-secret-leak"), "{message}");
}

#[test]
fn a_body_at_the_size_limit_decodes() {
    let body = body_of_size(MAX_CREDENTIAL_BODY_BYTES);
    drop(decode_credential_request::<CreateCredentialItem>(&body).unwrap());
}

#[test]
fn a_body_over_the_size_limit_is_refused_before_parsing() {
    let body = body_of_size(MAX_CREDENTIAL_BODY_BYTES + 1);
    let result = decode_credential_request::<CreateCredentialItem>(&body);
    assert!(matches!(result, Err(CredentialError::BodyTooLarge)));
}

#[test]
fn an_empty_body_is_refused() {
    let result = decode_credential_request::<UpdateCredentialItem>(b"");
    assert!(matches!(result, Err(CredentialError::Body { .. })));
}

#[rstest]
fn replays_the_credential_request_corpus(
    #[files("fuzz/corpus/credential_request/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("the corpus file is readable");
    drop(decode_credential_request::<CreateCredentialItem>(&bytes));
    drop(decode_credential_request::<UpdateCredentialItem>(&bytes));
}
