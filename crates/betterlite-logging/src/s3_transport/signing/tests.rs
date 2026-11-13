#![cfg(test)]

use std::time::{Duration, SystemTime};

use secrecy::SecretString;

use crate::s3_transport::download::EMPTY_SHA256;
use crate::s3_transport::signing::{
    Clock as _, Credentials, RequestToSign, SystemClock, sign_request,
};

const BODY_SHA256: &str = "2fd9a709a1c8f1321c0ebd69beb117fa7b3f4f6978ef268b1389fca6d20b5b35";
const PUT_URL: &str = "http://127.0.0.1:9000/logs/2026-01-02/id.json";
const PUT_AUTHORIZATION: &str = "AWS4-HMAC-SHA256 Credential=AKIAEXAMPLE/20260102/us-east-1/s3/aws4_request, SignedHeaders=cache-control;content-disposition;content-language;content-md5;content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token;x-amz-server-side-encryption;x-amz-server-side-encryption-aws-kms-key-id, Signature=48ba162a8b777dd7bcd7826480fcab9d6195de43c5c6486902800481df4c1656";
const GET_AUTHORIZATION: &str = "AWS4-HMAC-SHA256 Credential=AKIAEXAMPLE/20260102/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token, Signature=f24fb455950541be31ae93e0497584533d63b3910397a4b7483a53810cb0b5c4";

fn fixed_time() -> SystemTime {
    let stamp =
        chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z").expect("the time parses");
    let seconds = u64::try_from(stamp.timestamp()).expect("the time is after the epoch");
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .expect("the time fits")
}

fn credentials() -> Credentials {
    Credentials {
        access_key_id: SecretString::from("AKIAEXAMPLE"),
        secret_access_key: SecretString::from("secretKeyEXAMPLE"),
        session_token: Some(SecretString::from("tokenEXAMPLE")),
    }
}

fn put_headers() -> Vec<(String, String)> {
    [
        ("Content-Type", "application/json"),
        ("Content-MD5", "u+Mww//GWRAKyc7iFpGvNg=="),
        ("x-amz-content-sha256", BODY_SHA256),
        ("Content-Language", "en"),
        ("Content-Disposition", "inline; filename=\"download.json\""),
        (
            "Cache-Control",
            "private, immutable, max-age=31536000, s-maxage=0",
        ),
        ("x-amz-server-side-encryption", "aws:kms"),
        ("x-amz-server-side-encryption-aws-kms-key-id", "key-id"),
    ]
    .iter()
    .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
    .collect()
}

fn header<'headers>(headers: &'headers [(String, String)], name: &str) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name == name)
        .map(|(_, value)| value.as_str())
}

#[test]
fn the_system_clock_reads_the_current_time() {
    let before = SystemTime::now()
        .checked_sub(Duration::from_secs(1))
        .expect("the clock fits");
    let now = SystemClock.now();
    assert!(now >= before);
}

#[test]
fn a_put_signature_matches_botocore() {
    let headers = put_headers();
    let signed = sign_request(
        &RequestToSign {
            method: "PUT",
            url: PUT_URL,
            headers: &headers,
            content_sha256: BODY_SHA256,
            region: "us-east-1",
            now: fixed_time(),
        },
        &credentials(),
    )
    .expect("the request signs");
    assert_eq!(header(&signed, "authorization"), Some(PUT_AUTHORIZATION));
    assert_eq!(header(&signed, "x-amz-date"), Some("20260102T030405Z"));
    assert_eq!(
        header(&signed, "x-amz-security-token"),
        Some("tokenEXAMPLE")
    );
    assert_eq!(signed.len(), 3);
}

#[test]
fn a_get_signature_matches_botocore() {
    let headers = vec![("x-amz-content-sha256".to_owned(), EMPTY_SHA256.to_owned())];
    let signed = sign_request(
        &RequestToSign {
            method: "GET",
            url: PUT_URL,
            headers: &headers,
            content_sha256: EMPTY_SHA256,
            region: "us-east-1",
            now: fixed_time(),
        },
        &credentials(),
    )
    .expect("the request signs");
    assert_eq!(header(&signed, "authorization"), Some(GET_AUTHORIZATION));
    assert_eq!(signed.len(), 3);
}

#[test]
fn credentials_without_a_session_token_send_no_token_header() {
    let headers = vec![("x-amz-content-sha256".to_owned(), EMPTY_SHA256.to_owned())];
    let static_credentials = Credentials {
        access_key_id: SecretString::from("AKIAEXAMPLE"),
        secret_access_key: SecretString::from("secretKeyEXAMPLE"),
        session_token: None,
    };
    let signed = sign_request(
        &RequestToSign {
            method: "GET",
            url: PUT_URL,
            headers: &headers,
            content_sha256: EMPTY_SHA256,
            region: "us-east-1",
            now: fixed_time(),
        },
        &static_credentials,
    )
    .expect("the request signs");
    assert_eq!(header(&signed, "x-amz-security-token"), None);
    assert_eq!(signed.len(), 2);
}

#[test]
fn the_signed_payload_hash_changes_the_signature() {
    let signed = |content_sha256: &str| {
        sign_request(
            &RequestToSign {
                method: "PUT",
                url: PUT_URL,
                headers: &put_headers(),
                content_sha256,
                region: "us-east-1",
                now: fixed_time(),
            },
            &credentials(),
        )
        .expect("the request signs")
    };
    let one = signed(BODY_SHA256);
    let two = signed(EMPTY_SHA256);
    assert_ne!(one, two);
}

#[test]
fn the_signed_path_is_not_normalized() {
    let sign_path = |url: &str| {
        sign_request(
            &RequestToSign {
                method: "GET",
                url,
                headers: &[("x-amz-content-sha256".to_owned(), EMPTY_SHA256.to_owned())],
                content_sha256: EMPTY_SHA256,
                region: "us-east-1",
                now: fixed_time(),
            },
            &credentials(),
        )
        .expect("the request signs")
    };
    let with_dot_segment = sign_path("http://127.0.0.1:9000/logs/./a.json");
    let without_dot_segment = sign_path("http://127.0.0.1:9000/logs/a.json");
    assert_ne!(with_dot_segment, without_dot_segment);
}
