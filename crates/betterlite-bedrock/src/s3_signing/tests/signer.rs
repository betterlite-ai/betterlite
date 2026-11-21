use std::cell::Cell;
use std::collections::BTreeMap;
use std::error::Error as _;
use std::future::{Future, Ready, ready};
use std::time::{Duration, UNIX_EPOCH};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::runtime::Builder;

use super::support::{ENV_ACCESS_KEY, chain_credentials};
use crate::aws_settings::sources::{Credentials, SourceError};
use crate::credentials::error::CredentialError;
use crate::s3_signing::S3SigningError;
use crate::s3_signing::signer::{S3PutRequest, sign_s3_request};
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::RegionInputs;

const PATH_URL: &str = "https://s3.us-west-2.amazonaws.com/bkt/k";
const SIGNED: &str = "cache-control;content-language;content-length;content-type;host;x-amz-content-sha256;x-amz-date";
const X_HASH: &str = "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881";
const S3_PAIR: &str = r#"{"s3_access_key_id": "S3K", "s3_secret_access_key": "S3S"}"#;

struct Upload<'text> {
    params: &'text str,
    content: &'text str,
    api_base: &'text str,
    key: Option<&'text str>,
}

fn sign_with<F, Fut>(upload: &Upload<'_>, resolve: F) -> Result<AwsSignedRequest, S3SigningError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let optional_params: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(upload.params).unwrap();
    let key = upload.key.map(|key| SecretString::from(key.to_owned()));
    let request = S3PutRequest {
        content: upload.content,
        api_base: upload.api_base,
        optional_params: &optional_params,
        encryption_key_id: key.as_ref(),
        region_inputs: RegionInputs::default(),
        now: UNIX_EPOCH
            .checked_add(Duration::from_secs(1_704_067_200))
            .unwrap(),
    };
    let runtime = Builder::new_current_thread().build().unwrap();
    runtime.block_on(sign_s3_request(request, resolve))
}

fn sign(params: &str, content: &str) -> AwsSignedRequest {
    let upload = Upload {
        params,
        content,
        api_base: PATH_URL,
        key: None,
    };
    sign_with(&upload, chain_credentials).unwrap()
}

fn unreachable_chain(
    _settings: &AwsSettings,
    _region: &str,
) -> Ready<Result<Credentials, CredentialError>> {
    panic!("the s3_* key pair signs without the credential chain")
}

fn header_pairs(request: &AwsSignedRequest) -> Vec<(&str, &str)> {
    request
        .headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

fn authorization(request: &AwsSignedRequest) -> &str {
    request
        .headers
        .iter()
        .find(|(name, _)| name == "Authorization")
        .map_or("", |(_, value)| value.as_str())
}

fn expected<'text>(
    middle: &[(&'text str, &'text str)],
    signed: &[(&'text str, &'text str)],
) -> Vec<(&'text str, &'text str)> {
    let mut headers = vec![
        ("Content-Type", "application/json"),
        ("Content-Language", "en"),
        (
            "Cache-Control",
            "private, immutable, max-age=31536000, s-maxage=0",
        ),
    ];
    headers.extend_from_slice(middle);
    headers.push(("X-Amz-Date", "20240101T000000Z"));
    headers.extend_from_slice(signed);
    headers
}

pub(super) fn s3_signature_service() {
    let environment = sign("{}", "x");
    let authorization_env = format!(
        "AWS4-HMAC-SHA256 Credential={ENV_ACCESS_KEY}/20240101/us-west-2/s3/aws4_request, SignedHeaders={SIGNED}, Signature=16586e07f2370e8e8a42010dc1aab4bf4fc4058fbd2266acf722070a5e8e0164"
    );
    let expected_env = expected(
        &[("Content-Length", "1")],
        &[
            ("X-Amz-Content-SHA256", X_HASH),
            ("Authorization", authorization_env.as_str()),
        ],
    );
    assert_eq!(header_pairs(&environment), expected_env);
    assert_eq!(environment.method, "PUT");
    assert_eq!(environment.url, PATH_URL);
    assert_eq!(environment.body, b"x");
    let session_params = r#"{"aws_access_key_id": "AKIDSESSION", "aws_secret_access_key": "sessionsecret", "aws_session_token": "token-1", "aws_region_name": "us-east-1"}"#;
    let session = sign(session_params, "x");
    let authorization_session = format!(
        "AWS4-HMAC-SHA256 Credential=AKIDSESSION/20240101/us-east-1/s3/aws4_request, SignedHeaders={SIGNED};x-amz-security-token, Signature=10821dbdfddfa9f007b25972f8e82b1d2ead281a8346697ac6ba599a0dd60c7e"
    );
    let expected_session = expected(
        &[("Content-Length", "1")],
        &[
            ("X-Amz-Security-Token", "token-1"),
            ("X-Amz-Content-SHA256", X_HASH),
            ("Authorization", authorization_session.as_str()),
        ],
    );
    assert_eq!(header_pairs(&session), expected_session);
    let virtual_host = Upload {
        params: r#"{"s3_access_key_id": "S3K", "s3_secret_access_key": "S3S", "aws_region_name": "eu-west-1"}"#,
        content: "x",
        api_base: "https://bkt.s3.eu-west-1.amazonaws.com/a%20b/c.jsonl?x=1",
        key: None,
    };
    let hosted = sign_with(&virtual_host, unreachable_chain).unwrap();
    assert_eq!(
        authorization(&hosted),
        format!(
            "AWS4-HMAC-SHA256 Credential=S3K/20240101/eu-west-1/s3/aws4_request, SignedHeaders={SIGNED}, Signature=52401e901a7b24d030bfcfaf0dcca2a8808fe66935e97b3623d68d2088ffdd5b"
        )
    );
    assert_eq!(hosted.url, virtual_host.api_base);
    // the content is signed and sent as its own UTF-8 bytes
    let accented = sign(S3_PAIR, "h\u{e9}llo\n{}");
    assert_eq!(accented.body, "h\u{e9}llo\n{}".as_bytes());
    assert!(header_pairs(&accented).contains(&("Content-Length", "9")));
    let bad_region = sign_with(
        &Upload {
            params: r#"{"aws_region_name": "Bad"}"#,
            ..virtual_host
        },
        chain_credentials,
    )
    .unwrap_err();
    assert_eq!(bad_region.to_string(), "invalid AWS region format: Bad");
}

pub(super) fn kms_encryption_header() {
    let upload = Upload {
        params: S3_PAIR,
        content: "h\u{e9}llo\n{}",
        api_base: PATH_URL,
        key: Some("key-1"),
    };
    let encrypted = sign_with(&upload, unreachable_chain).unwrap();
    let encrypted_authorization = format!(
        "AWS4-HMAC-SHA256 Credential=S3K/20240101/us-west-2/s3/aws4_request, SignedHeaders={SIGNED};x-amz-server-side-encryption;x-amz-server-side-encryption-aws-kms-key-id, Signature=526d0a97ca59476c34d04b09d16fd6732ac2e270e49a7c79c2b95427fd2475e4"
    );
    let middle = [
        ("x-amz-server-side-encryption", "aws:kms"),
        ("x-amz-server-side-encryption-aws-kms-key-id", "key-1"),
        ("Content-Length", "9"),
    ];
    let hash = "196cc5fc838686cf3ea5052f212f2478ab5aa694cfb01a501b99205e5a9dd724";
    assert_eq!(
        header_pairs(&encrypted),
        expected(
            &middle,
            &[
                ("X-Amz-Content-SHA256", hash),
                ("Authorization", encrypted_authorization.as_str())
            ]
        )
    );
    // an empty key adds neither header, and no owner header is ever signed here
    let unencrypted = sign_with(
        &Upload {
            content: "",
            key: Some(""),
            ..upload
        },
        unreachable_chain,
    )
    .unwrap();
    let empty_authorization = format!(
        "AWS4-HMAC-SHA256 Credential=S3K/20240101/us-west-2/s3/aws4_request, SignedHeaders={SIGNED}, Signature=78e64dd1a40d50176a55a7d8744cad5c2d4f0ae4ffc6fc9e24092033e05a1eeb"
    );
    let empty_hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    assert_eq!(
        header_pairs(&unencrypted),
        expected(
            &[("Content-Length", "0")],
            &[
                ("X-Amz-Content-SHA256", empty_hash),
                ("Authorization", empty_authorization.as_str())
            ]
        )
    );
    let without_key = sign_with(
        &Upload {
            content: "",
            ..upload
        },
        unreachable_chain,
    )
    .unwrap();
    assert_eq!(header_pairs(&without_key), header_pairs(&unencrypted));
    for request in [&encrypted, &unencrypted] {
        assert!(
            request
                .headers
                .iter()
                .all(|(name, _)| name != "x-amz-expected-bucket-owner")
        );
    }
}

#[test]
fn bedrock_services_s3_signing_credential_identity_isolation() {
    let first = sign(S3_PAIR, "x");
    let other = sign(
        r#"{"s3_access_key_id": "OTHER", "s3_secret_access_key": "S3S"}"#,
        "x",
    );
    assert_ne!(authorization(&first), authorization(&other));
    assert_eq!(
        authorization(&other),
        format!(
            "AWS4-HMAC-SHA256 Credential=OTHER/20240101/us-west-2/s3/aws4_request, SignedHeaders={SIGNED}, Signature=3fd0fadbd569f3f19d6c5bfb73401e6bed4c6e8deee2d729731043e73ef9d97f"
        )
    );
    assert!(authorization(&first).starts_with("AWS4-HMAC-SHA256 Credential=S3K/"));
    // a request with its own aws_* keys leaves nothing behind for the next request without keys
    let session = sign(
        r#"{"aws_access_key_id": "AKIDSESSION", "aws_secret_access_key": "sessionsecret", "aws_session_token": "token-1"}"#,
        "x",
    );
    assert!(authorization(&session).starts_with("AWS4-HMAC-SHA256 Credential=AKIDSESSION/"));
    let environment = sign("{}", "x");
    assert!(
        authorization(&environment)
            .starts_with(&format!("AWS4-HMAC-SHA256 Credential={ENV_ACCESS_KEY}/"))
    );
    assert!(
        header_pairs(&environment)
            .iter()
            .all(|(name, _)| *name != "X-Amz-Security-Token")
    );
    // an incomplete s3_* pair is not an identity, so the chain answers
    let partial = sign(
        r#"{"s3_access_key_id": "S3K", "s3_secret_access_key": ""}"#,
        "x",
    );
    assert_eq!(authorization(&partial), authorization(&environment));
    let calls = Cell::new(0_u32);
    let counting = |settings: &AwsSettings, region: &str| {
        calls.set(calls.get().saturating_add(1));
        chain_credentials(settings, region)
    };
    let upload = Upload {
        params: S3_PAIR,
        content: "x",
        api_base: PATH_URL,
        key: None,
    };
    sign_with(&upload, counting).unwrap();
    assert_eq!(calls.get(), 0);
}

#[test]
fn bedrock_services_s3_signing_secret_safe_failure() {
    let secret_pair =
        r#"{"s3_access_key_id": "AKIDLEAK", "s3_secret_access_key": "leaked-secret-value"}"#;
    let invalid_url = Upload {
        params: secret_pair,
        content: "x",
        api_base: "not a url",
        key: Some("kms-secret-key"),
    };
    let failure = sign_with(&invalid_url, unreachable_chain).unwrap_err();
    assert!(matches!(failure, S3SigningError::Sign(_)));
    let failing_chain = |_settings: &AwsSettings, _region: &str| {
        ready(Err::<Credentials, CredentialError>(
            CredentialError::Source(SourceError::MissingProvider),
        ))
    };
    let chain_params =
        r#"{"aws_access_key_id": "AKIDLEAK", "aws_secret_access_key": "leaked-secret-value"}"#;
    let lookup = sign_with(
        &Upload {
            params: chain_params,
            ..invalid_url
        },
        failing_chain,
    )
    .unwrap_err();
    assert_eq!(lookup.to_string(), "the AWS credential lookup failed");
    let wrong_type = sign_with(
        &Upload {
            params: r#"{"s3_secret_access_key": ["leaked-secret-value"]}"#,
            ..invalid_url
        },
        unreachable_chain,
    )
    .unwrap_err();
    assert!(wrong_type.to_string().starts_with("s3_secret_access_key: "));
    for error in [&failure, &lookup, &wrong_type] {
        let mut parts = vec![format!("{error} {error:?}")];
        let mut source = error.source();
        while let Some(cause) = source {
            parts.push(format!("{cause} {cause:?}"));
            source = cause.source();
        }
        let rendered = parts.join(" ");
        for secret in ["AKIDLEAK", "leaked-secret-value", "kms-secret-key"] {
            assert!(!rendered.contains(secret), "{secret} leaked in {rendered}");
        }
    }
    let signed = sign(secret_pair, "x");
    let debug = format!("{signed:?}");
    assert!(
        !debug.contains("AKIDLEAK") && !debug.contains("Signature="),
        "{debug}"
    );
}
