#![cfg(test)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use secrecy::SecretString;

use crate::credentials::Credentials;
use crate::sign::{Signable, SigningMode, presign_request, sign_request};

const BODY_SHA256: &str = "2fd9a709a1c8f1321c0ebd69beb117fa7b3f4f6978ef268b1389fca6d20b5b35";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const PUT_URL: &str = "http://127.0.0.1:9000/logs/2026-01-02/id.json";
const PUT_AUTHORIZATION: &str = "AWS4-HMAC-SHA256 Credential=AKIAEXAMPLE/20260102/us-east-1/s3/aws4_request, SignedHeaders=cache-control;content-disposition;content-language;content-md5;content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token;x-amz-server-side-encryption;x-amz-server-side-encryption-aws-kms-key-id, Signature=48ba162a8b777dd7bcd7826480fcab9d6195de43c5c6486902800481df4c1656";
const GET_AUTHORIZATION: &str = "AWS4-HMAC-SHA256 Credential=AKIAEXAMPLE/20260102/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token, Signature=f24fb455950541be31ae93e0497584533d63b3910397a4b7483a53810cb0b5c4";
const ELASTICACHE_SIGNATURE: &str =
    "e1e4ce13c45cfa056cbbb7bf8235fe64d59d0f2b10176bea67739664779bcfd4";

// 2026-01-02T03:04:05Z, the time botocore signed the S3 answers below at
fn fixed_time() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_767_323_045))
        .expect("the fixed time is after the epoch")
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
fn s3_sigv4_auth_signs_a_put_like_botocore() {
    let headers = put_headers();
    let signed = sign_request(&Signable {
        method: "PUT",
        url: PUT_URL,
        headers: &headers,
        body: b"{\"id\":\"request\"}",
        service: "s3",
        region: "us-east-1",
        credentials: &credentials(),
        mode: SigningMode::S3SigV4Auth,
        now: fixed_time(),
    })
    .expect("the request signs");
    assert_eq!(header(&signed, "Authorization"), Some(PUT_AUTHORIZATION));
    assert_eq!(header(&signed, "X-Amz-Date"), Some("20260102T030405Z"));
    assert_eq!(
        header(&signed, "X-Amz-Security-Token"),
        Some("tokenEXAMPLE")
    );
    assert_eq!(signed.len(), 3);
}

#[test]
fn s3_sigv4_auth_signs_a_get_like_botocore() {
    let headers = vec![("x-amz-content-sha256".to_owned(), EMPTY_SHA256.to_owned())];
    let signed = sign_request(&Signable {
        method: "GET",
        url: PUT_URL,
        headers: &headers,
        body: &[],
        service: "s3",
        region: "us-east-1",
        credentials: &credentials(),
        mode: SigningMode::S3SigV4Auth,
        now: fixed_time(),
    })
    .expect("the request signs");
    assert_eq!(header(&signed, "Authorization"), Some(GET_AUTHORIZATION));
    assert_eq!(signed.len(), 3);
}

#[test]
fn s3_sigv4_auth_sends_no_token_without_one() {
    let headers = vec![("x-amz-content-sha256".to_owned(), EMPTY_SHA256.to_owned())];
    let static_credentials = Credentials {
        access_key_id: SecretString::from("AKIAEXAMPLE"),
        secret_access_key: SecretString::from("secretKeyEXAMPLE"),
        session_token: None,
    };
    let signed = sign_request(&Signable {
        method: "GET",
        url: PUT_URL,
        headers: &headers,
        body: &[],
        service: "s3",
        region: "us-east-1",
        credentials: &static_credentials,
        mode: SigningMode::S3SigV4Auth,
        now: fixed_time(),
    })
    .expect("the request signs");
    assert_eq!(header(&signed, "X-Amz-Security-Token"), None);
    assert_eq!(signed.len(), 2);
}

#[test]
fn s3_sigv4_auth_signs_the_payload_hash_header() {
    let signed = |content_sha256: &str| {
        let headers: Vec<(String, String)> = put_headers()
            .into_iter()
            .map(|(name, value)| {
                if name.eq_ignore_ascii_case("x-amz-content-sha256") {
                    (name, content_sha256.to_owned())
                } else {
                    (name, value)
                }
            })
            .collect();
        sign_request(&Signable {
            method: "PUT",
            url: PUT_URL,
            headers: &headers,
            body: &[],
            service: "s3",
            region: "us-east-1",
            credentials: &credentials(),
            mode: SigningMode::S3SigV4Auth,
            now: fixed_time(),
        })
        .expect("the request signs")
    };
    assert_ne!(signed(BODY_SHA256), signed(EMPTY_SHA256));
}

#[test]
fn s3_sigv4_auth_signs_the_path_as_sent() {
    let sign_path = |url: &str| {
        sign_request(&Signable {
            method: "GET",
            url,
            headers: &[("x-amz-content-sha256".to_owned(), EMPTY_SHA256.to_owned())],
            body: &[],
            service: "s3",
            region: "us-east-1",
            credentials: &credentials(),
            mode: SigningMode::S3SigV4Auth,
            now: fixed_time(),
        })
        .expect("the request signs")
    };
    let with_dot_segment = sign_path("http://127.0.0.1:9000/logs/./a.json");
    let without_dot_segment = sign_path("http://127.0.0.1:9000/logs/a.json");
    assert_ne!(with_dot_segment, without_dot_segment);
}

fn elasticache_credentials() -> Credentials {
    Credentials {
        access_key_id: SecretString::from("AKIAIOSFODNN7EXAMPLE"),
        secret_access_key: SecretString::from("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
        session_token: None,
    }
}

fn presigned(credentials: &Credentials, url: &str, expires_in: u64) -> Vec<(String, String)> {
    presign_request(
        &Signable {
            method: "GET",
            url,
            headers: &[],
            body: &[],
            service: "elasticache",
            region: "eu-west-1",
            credentials,
            mode: SigningMode::SigV4Auth,
            now: UNIX_EPOCH,
        },
        Duration::from_secs(expires_in),
    )
    .expect("the request presigns")
}

fn parameter<'parameters>(
    parameters: &'parameters [(String, String)],
    name: &str,
) -> Option<&'parameters str> {
    parameters
        .iter()
        .find(|(parameter_name, _)| parameter_name == name)
        .map(|(_, value)| value.as_str())
}

#[test]
fn presign_matches_the_botocore_elasticache_answer() {
    let parameters = presigned(
        &elasticache_credentials(),
        "https://cache.example/?Action=connect&User=app",
        900,
    );
    let expected = [
        ("X-Amz-Algorithm", "AWS4-HMAC-SHA256"),
        (
            "X-Amz-Credential",
            "AKIAIOSFODNN7EXAMPLE/19700101/eu-west-1/elasticache/aws4_request",
        ),
        ("X-Amz-Date", "19700101T000000Z"),
        ("X-Amz-Expires", "900"),
        ("X-Amz-SignedHeaders", "host"),
        ("X-Amz-Signature", ELASTICACHE_SIGNATURE),
    ];
    let pairs: Vec<(&str, &str)> = parameters
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(pairs, expected);
}

#[test]
fn presign_puts_the_security_token_before_the_signature() {
    let with_token = Credentials {
        access_key_id: SecretString::from("AKIAIOSFODNN7EXAMPLE"),
        secret_access_key: SecretString::from("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
        session_token: Some(SecretString::from("tokenEXAMPLE")),
    };
    let parameters = presigned(
        &with_token,
        "https://cache.example/?Action=connect&User=app",
        900,
    );
    let names: Vec<&str> = parameters.iter().map(|(name, _)| name.as_str()).collect();
    let token = names
        .iter()
        .position(|name| *name == "X-Amz-Security-Token")
        .expect("the token is listed");
    let signature = names
        .iter()
        .position(|name| *name == "X-Amz-Signature")
        .expect("the signature is listed");
    assert!(token < signature);
    assert_eq!(signature, names.len().saturating_sub(1));
    assert_eq!(
        parameter(&parameters, "X-Amz-Security-Token"),
        Some("tokenEXAMPLE")
    );
}

#[test]
fn presign_signs_the_expiry() {
    let credentials = elasticache_credentials();
    let url = "https://cache.example/?Action=connect&User=app";
    let short = presigned(&credentials, url, 300);
    let long = presigned(&credentials, url, 900);
    assert_eq!(parameter(&short, "X-Amz-Expires"), Some("300"));
    assert_eq!(parameter(&long, "X-Amz-Expires"), Some("900"));
    assert_ne!(
        parameter(&short, "X-Amz-Signature"),
        parameter(&long, "X-Amz-Signature")
    );
}
