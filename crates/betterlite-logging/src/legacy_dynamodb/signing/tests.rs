#![cfg(test)]

use std::time::SystemTime;

use aws_credential_types::Credentials;

use super::{Signable, signed_headers};
use crate::sqs_logging::tests::support::{ACCESS_KEY_ID, SECRET_ACCESS_KEY, fixed_time};

const URL: &str = "https://dynamodb.us-east-1.amazonaws.com/";
const DATE_STAMP: &str = "20260102T030405Z";

fn credentials(token: Option<&str>) -> Credentials {
    Credentials::new(
        ACCESS_KEY_ID,
        SECRET_ACCESS_KEY,
        token.map(str::to_owned),
        None,
        "test",
    )
}

fn headers() -> Vec<(String, String)> {
    vec![
        (
            "content-type".to_owned(),
            "application/x-amz-json-1.0".to_owned(),
        ),
        (
            "x-amz-target".to_owned(),
            "DynamoDB_20120810.PutItem".to_owned(),
        ),
    ]
}

fn signed(region: &str, body: &[u8], now: SystemTime) -> Vec<(String, String)> {
    let unsigned = headers();
    signed_headers(
        &Signable {
            method: "POST",
            url: URL,
            headers: &unsigned,
            body,
            region,
            now,
        },
        &credentials(None),
    )
    .expect("the request signs")
}

fn value<'headers>(headers: &'headers [(String, String)], name: &str) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(header, _value)| header == name)
        .map(|(_header, value)| value.as_str())
}

#[test]
fn signs_the_dynamodb_service_and_region() {
    let headers = signed("us-east-1", b"{}", fixed_time());
    let authorization = value(&headers, "authorization").expect("the request is signed");
    assert!(authorization.starts_with(&format!(
        "AWS4-HMAC-SHA256 Credential={ACCESS_KEY_ID}/20260102/us-east-1/dynamodb/aws4_request"
    )));
    assert_eq!(value(&headers, "x-amz-date"), Some(DATE_STAMP));
}

#[test]
fn carries_the_session_token() {
    let unsigned = headers();
    let headers = signed_headers(
        &Signable {
            method: "POST",
            url: URL,
            headers: &unsigned,
            body: b"{}",
            region: "us-east-1",
            now: fixed_time(),
        },
        &credentials(Some("SESSIONTOKEN")),
    )
    .expect("the request signs");
    assert_eq!(
        value(&headers, "x-amz-security-token"),
        Some("SESSIONTOKEN")
    );
}

#[test]
fn a_different_body_changes_the_signature() {
    let first = signed("us-east-1", b"{}", fixed_time());
    let second = signed("us-east-1", b"{\"TableName\":\"logs\"}", fixed_time());
    assert_ne!(
        value(&first, "authorization"),
        value(&second, "authorization")
    );
}

#[test]
fn a_different_region_changes_the_signature() {
    let first = signed("us-east-1", b"{}", fixed_time());
    let second = signed("eu-west-1", b"{}", fixed_time());
    assert_ne!(
        value(&first, "authorization"),
        value(&second, "authorization")
    );
    assert!(
        value(&second, "authorization")
            .expect("the request is signed")
            .contains("/eu-west-1/dynamodb/aws4_request")
    );
}
