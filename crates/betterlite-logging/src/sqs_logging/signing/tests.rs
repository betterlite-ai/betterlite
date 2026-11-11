#![cfg(test)]

use aws_credential_types::Credentials;
use http::header::CONTENT_TYPE;
use http::{Method, Request};

use crate::sqs_logging::message::FORM_CONTENT_TYPE;
use crate::sqs_logging::tests::support::{ACCESS_KEY_ID, QUEUE_URL, SECRET_ACCESS_KEY, fixed_time};

use super::sign_request;

const BODY: &str = "Action=SendMessage&Version=2012-11-05&MessageBody=test-body";
const BOTOCORE_AUTHORIZATION: &str = "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20260102/us-east-1/sqs/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=fb20ab3a9cf9925d4736994fd0151689c0fab2c7f45bab185ae3d6a9f7126ffb";
const BOTOCORE_SESSION_AUTHORIZATION: &str = "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20260102/us-east-1/sqs/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token, Signature=6828413e1fa1615a8f69196f1d6d5c3f79b0fb1398df106fc4144d850d72d924";

fn credentials(session_token: Option<&str>) -> Credentials {
    Credentials::new(
        ACCESS_KEY_ID,
        SECRET_ACCESS_KEY,
        session_token.map(str::to_owned),
        None,
        "test",
    )
}

fn request() -> Request<Vec<u8>> {
    Request::builder()
        .method(Method::POST)
        .uri(QUEUE_URL)
        .header(CONTENT_TYPE, FORM_CONTENT_TYPE)
        .body(BODY.as_bytes().to_vec())
        .expect("the request builds")
}

fn header<'request>(request: &'request Request<Vec<u8>>, name: &str) -> &'request str {
    request
        .headers()
        .get(name)
        .expect("the signed header is present")
        .to_str()
        .expect("the signed header is text")
}

#[test]
fn signs_the_request_like_botocore() {
    let mut request = request();
    sign_request(&mut request, &credentials(None), "us-east-1", fixed_time())
        .expect("the request signs");
    assert_eq!(header(&request, "authorization"), BOTOCORE_AUTHORIZATION);
    assert_eq!(header(&request, "x-amz-date"), "20260102T030405Z");
    assert_eq!(request.body(), BODY.as_bytes());
}

#[test]
fn signing_adds_the_session_token() {
    let mut request = request();
    sign_request(
        &mut request,
        &credentials(Some("SESSIONTOKEN")),
        "us-east-1",
        fixed_time(),
    )
    .expect("the request signs");
    assert_eq!(
        header(&request, "authorization"),
        BOTOCORE_SESSION_AUTHORIZATION
    );
    assert_eq!(header(&request, "x-amz-security-token"), "SESSIONTOKEN");
}

#[test]
fn signing_omits_the_session_token_without_one() {
    let mut request = request();
    sign_request(&mut request, &credentials(None), "us-east-1", fixed_time())
        .expect("the request signs");
    assert!(request.headers().get("x-amz-security-token").is_none());
}

#[test]
fn the_signature_covers_the_request_body() {
    let mut first = request();
    sign_request(&mut first, &credentials(None), "us-east-1", fixed_time())
        .expect("the request signs");
    let mut second = Request::builder()
        .method(Method::POST)
        .uri(QUEUE_URL)
        .header(CONTENT_TYPE, FORM_CONTENT_TYPE)
        .body(BODY.replace("test-body", "other-body").into_bytes())
        .expect("the request builds");
    sign_request(&mut second, &credentials(None), "us-east-1", fixed_time())
        .expect("the request signs");
    assert_ne!(
        header(&first, "authorization"),
        header(&second, "authorization")
    );
}
