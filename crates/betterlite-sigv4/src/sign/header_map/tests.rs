#![cfg(test)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_credential_types::Credentials as AwsCredentials;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Method, Request};
use rstest::rstest;

use super::{insert_signed, sign_in_place};
use crate::credentials::Credentials;
use crate::error::SigV4Error;

const QUEUE_URL: &str = "https://sqs.us-east-1.amazonaws.com/123456789012/test-queue";
const ACCESS_KEY_ID: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET_ACCESS_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const BODY: &str = "Action=SendMessage&Version=2012-11-05&MessageBody=test-body";
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";
const BOTOCORE_AUTHORIZATION: &str = "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20260102/us-east-1/sqs/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=fb20ab3a9cf9925d4736994fd0151689c0fab2c7f45bab185ae3d6a9f7126ffb";
const BOTOCORE_SESSION_AUTHORIZATION: &str = "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20260102/us-east-1/sqs/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token, Signature=6828413e1fa1615a8f69196f1d6d5c3f79b0fb1398df106fc4144d850d72d924";

fn fixed_time() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_767_323_045))
        .expect("the fixed time is representable")
}

fn credentials(session_token: Option<&str>) -> Credentials {
    let aws = AwsCredentials::new(
        ACCESS_KEY_ID,
        SECRET_ACCESS_KEY,
        session_token.map(str::to_owned),
        None,
        "test",
    );
    Credentials::from(&aws)
}

fn request() -> Request<Vec<u8>> {
    Request::builder()
        .method(Method::POST)
        .uri(QUEUE_URL)
        .header(CONTENT_TYPE, FORM_CONTENT_TYPE)
        .body(BODY.as_bytes().to_vec())
        .expect("the request builds")
}

fn header<'request>(request: &'request Request<Vec<u8>>, name: &str) -> Option<&'request str> {
    request
        .headers()
        .get(name)
        .map(|value| value.to_str().expect("the signed header is text"))
}

#[test]
fn signs_an_sqs_post_like_botocore() {
    let mut request = request();
    sign_in_place(
        &mut request,
        &credentials(None),
        "sqs",
        "us-east-1",
        fixed_time(),
    )
    .expect("the request signs");
    assert_eq!(
        header(&request, "authorization"),
        Some(BOTOCORE_AUTHORIZATION)
    );
    assert_eq!(header(&request, "x-amz-date"), Some("20260102T030405Z"));
    assert_eq!(request.body(), BODY.as_bytes());
}

#[test]
fn signs_an_sqs_post_with_a_session_token_like_botocore() {
    let mut request = request();
    sign_in_place(
        &mut request,
        &credentials(Some("SESSIONTOKEN")),
        "sqs",
        "us-east-1",
        fixed_time(),
    )
    .expect("the request signs");
    assert_eq!(
        header(&request, "authorization"),
        Some(BOTOCORE_SESSION_AUTHORIZATION)
    );
    assert_eq!(
        header(&request, "x-amz-security-token"),
        Some("SESSIONTOKEN")
    );
}

#[rstest]
#[case::without_a_session_token(false)]
#[case::with_a_session_token(true)]
fn replaces_stale_signing_headers(#[case] has_session_token: bool) {
    let mut request = request();
    request
        .headers_mut()
        .insert(AUTHORIZATION, HeaderValue::from_static("Bearer stale"));
    request
        .headers_mut()
        .insert("x-amz-date", HeaderValue::from_static("20000101T000000Z"));
    request.headers_mut().insert(
        "x-amz-security-token",
        HeaderValue::from_static("stale-token"),
    );
    let token = has_session_token.then_some("SESSIONTOKEN");
    sign_in_place(
        &mut request,
        &credentials(token),
        "sqs",
        "us-east-1",
        fixed_time(),
    )
    .expect("the request signs");
    assert!(
        header(&request, "authorization")
            .expect("the request is signed")
            .starts_with("AWS4-HMAC-SHA256 ")
    );
    assert_eq!(request.headers().get_all(AUTHORIZATION).iter().count(), 1);
    assert_eq!(header(&request, "x-amz-date"), Some("20260102T030405Z"));
    let expected_token = if has_session_token {
        "SESSIONTOKEN"
    } else {
        "stale-token"
    };
    assert_eq!(
        header(&request, "x-amz-security-token"),
        Some(expected_token)
    );
}

#[test]
fn joins_repeated_header_values() {
    let mut repeated = request();
    repeated
        .headers_mut()
        .append("x-tag", HeaderValue::from_static("a"));
    repeated
        .headers_mut()
        .append("x-tag", HeaderValue::from_static("b"));
    let mut joined = request();
    joined
        .headers_mut()
        .insert("x-tag", HeaderValue::from_static("a, b"));
    sign_in_place(
        &mut repeated,
        &credentials(None),
        "sqs",
        "us-east-1",
        fixed_time(),
    )
    .expect("the repeated request signs");
    sign_in_place(
        &mut joined,
        &credentials(None),
        "sqs",
        "us-east-1",
        fixed_time(),
    )
    .expect("the joined request signs");
    assert_eq!(
        header(&repeated, "authorization"),
        header(&joined, "authorization")
    );
}

#[test]
fn marks_every_signed_header_but_the_date_sensitive() {
    let mut headers = HeaderMap::new();
    let signed = vec![
        (
            "Authorization".to_owned(),
            "AWS4-HMAC-SHA256 test".to_owned(),
        ),
        ("X-Amz-Date".to_owned(), "20260102T030405Z".to_owned()),
        ("X-Amz-Security-Token".to_owned(), "SESSIONTOKEN".to_owned()),
    ];
    insert_signed(&mut headers, signed).expect("the signed headers insert");
    assert!(headers["authorization"].is_sensitive());
    assert!(!headers["x-amz-date"].is_sensitive());
    assert!(headers["x-amz-security-token"].is_sensitive());
    assert_eq!(
        headers["x-amz-date"].to_str().expect("the date is text"),
        "20260102T030405Z"
    );
}

#[test]
fn refuses_a_header_value_that_is_not_text() {
    let mut request = request();
    request.headers_mut().insert(
        "x-bytes",
        HeaderValue::from_bytes(&[0xE9]).expect("the raw value is a header value"),
    );
    let error = sign_in_place(
        &mut request,
        &credentials(None),
        "sqs",
        "us-east-1",
        fixed_time(),
    )
    .expect_err("the header value is not visible ASCII");
    assert!(matches!(error, SigV4Error::HeaderText { name, .. } if name == "x-bytes"));
}
