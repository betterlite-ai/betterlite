use std::cell::Cell;
use std::collections::BTreeMap;
use std::error::Error as _;
use std::future::{Future, ready};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::runtime::{Builder, Runtime};

use super::support::{ENV_ACCESS_KEY, chain_credentials};
use crate::aws_settings::sources::{Credentials, SourceError};
use crate::credentials::error::CredentialError;
use crate::s3_signing::S3SigningError;
use crate::s3_signing::signer::{S3PutRequest, sign_s3_request};
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::RegionInputs;

const PATH_URL: &str = "https://s3.us-west-2.amazonaws.com/bkt/k";
const SIGNED: &str = "cache-control;content-language;content-length;content-type;host;x-amz-content-sha256;x-amz-date";
const SSE: &str = ";x-amz-server-side-encryption;x-amz-server-side-encryption-aws-kms-key-id";
const X_HASH: &str = "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881";
const EMPTY_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const ACCENTED_HASH: &str = "196cc5fc838686cf3ea5052f212f2478ab5aa694cfb01a501b99205e5a9dd724";
const S3_PAIR: &str = r#"{"s3_access_key_id": "S3K", "s3_secret_access_key": "S3S"}"#;
const SESSION: &str = r#"{"aws_access_key_id": "AKIDSESSION", "aws_secret_access_key": "sessionsecret", "aws_session_token": "token-1", "aws_region_name": "us-east-1"}"#;

#[derive(Clone, Copy)]
struct Upload<'text> {
    params: &'text str,
    content: &'text str,
    api_base: &'text str,
    key: Option<&'text str>,
}

const KMS_UPLOAD: Upload<'static> = Upload {
    params: S3_PAIR,
    content: "h\u{e9}llo\n{}",
    api_base: PATH_URL,
    key: Some("key-1"),
};

struct Prepared {
    optional_params: BTreeMap<String, Box<RawValue>>,
    key: Option<SecretString>,
    runtime: Runtime,
    now: SystemTime,
}

fn prepare(upload: Upload<'_>) -> Prepared {
    let seconds = Duration::from_secs(1_704_067_200);
    Prepared {
        optional_params: serde_json::from_str(upload.params).unwrap(),
        key: upload.key.map(|key| SecretString::from(key.to_owned())),
        runtime: Builder::new_current_thread().build().unwrap(),
        now: UNIX_EPOCH.checked_add(seconds).unwrap(),
    }
}

fn sign_with<F, Fut>(upload: Upload<'_>, resolve: F) -> Result<AwsSignedRequest, S3SigningError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let prepared = prepare(upload);
    let request = S3PutRequest {
        content: upload.content,
        api_base: upload.api_base,
        optional_params: &prepared.optional_params,
        encryption_key_id: prepared.key.as_ref(),
        region_inputs: RegionInputs::default(),
        now: prepared.now,
    };
    prepared.runtime.block_on(sign_s3_request(request, resolve))
}

fn sign(params: &str, content: &str) -> AwsSignedRequest {
    let upload = Upload {
        params,
        content,
        api_base: PATH_URL,
        key: None,
    };
    sign_with(upload, chain_credentials).unwrap()
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

fn signature(scope: &str, extra: &str, value: &str) -> String {
    let credential = format!("Credential={scope}/s3/aws4_request");
    format!("AWS4-HMAC-SHA256 {credential}, SignedHeaders={SIGNED}{extra}, Signature={value}")
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
    chain_signatures();
    s3_pair_signatures();
}

fn chain_signatures() {
    let environment = sign("{}", "x");
    let scope = format!("{ENV_ACCESS_KEY}/20240101/us-west-2");
    let env_value = "16586e07f2370e8e8a42010dc1aab4bf4fc4058fbd2266acf722070a5e8e0164";
    let env_auth = signature(&scope, "", env_value);
    let length = [("Content-Length", "1")];
    let env_signed = [
        ("X-Amz-Content-SHA256", X_HASH),
        ("Authorization", env_auth.as_str()),
    ];
    assert_eq!(header_pairs(&environment), expected(&length, &env_signed));
    assert_eq!(environment.method, "PUT");
    assert_eq!(environment.url, PATH_URL);
    assert_eq!(environment.body, b"x");
    let session = sign(SESSION, "x");
    let session_value = "10821dbdfddfa9f007b25972f8e82b1d2ead281a8346697ac6ba599a0dd60c7e";
    let session_scope = "AKIDSESSION/20240101/us-east-1";
    let session_auth = signature(session_scope, ";x-amz-security-token", session_value);
    let session_signed = [
        ("X-Amz-Security-Token", "token-1"),
        ("X-Amz-Content-SHA256", X_HASH),
        ("Authorization", session_auth.as_str()),
    ];
    assert_eq!(header_pairs(&session), expected(&length, &session_signed));
    let bad_region = Upload {
        params: r#"{"aws_region_name": "Bad"}"#,
        content: "x",
        api_base: PATH_URL,
        key: None,
    };
    let error = sign_with(bad_region, chain_credentials).unwrap_err();
    assert_eq!(error.to_string(), "invalid AWS region format: Bad");
}

fn s3_pair_signatures() {
    let hosted_params = r#"{"s3_access_key_id": "S3K", "s3_secret_access_key": "S3S", "aws_region_name": "eu-west-1"}"#;
    let api_base = "https://bkt.s3.eu-west-1.amazonaws.com/a%20b/c.jsonl?x=1";
    let hosted_upload = Upload {
        params: hosted_params,
        content: "x",
        api_base,
        key: None,
    };
    let hosted = sign_with(hosted_upload, chain_credentials).unwrap();
    let hosted_value = "52401e901a7b24d030bfcfaf0dcca2a8808fe66935e97b3623d68d2088ffdd5b";
    let hosted_auth = signature("S3K/20240101/eu-west-1", "", hosted_value);
    assert_eq!(authorization(&hosted), hosted_auth);
    assert_eq!(hosted.url, api_base);
    // the content is signed and sent as its own UTF-8 bytes
    let accented = sign(S3_PAIR, "h\u{e9}llo\n{}");
    assert_eq!(accented.body, "h\u{e9}llo\n{}".as_bytes());
    let pairs = header_pairs(&accented);
    assert!(pairs.contains(&("Content-Length", "9")));
    assert!(pairs.contains(&("X-Amz-Content-SHA256", ACCENTED_HASH)));
}

pub(super) fn kms_encryption_header() {
    let encrypted = sign_with(KMS_UPLOAD, chain_credentials).unwrap();
    let encrypted_value = "526d0a97ca59476c34d04b09d16fd6732ac2e270e49a7c79c2b95427fd2475e4";
    let encrypted_auth = signature("S3K/20240101/us-west-2", SSE, encrypted_value);
    let middle = [
        ("x-amz-server-side-encryption", "aws:kms"),
        ("x-amz-server-side-encryption-aws-kms-key-id", "key-1"),
        ("Content-Length", "9"),
    ];
    let signed = [
        ("X-Amz-Content-SHA256", ACCENTED_HASH),
        ("Authorization", encrypted_auth.as_str()),
    ];
    // the exact header list also proves no expected-bucket-owner header is signed
    assert_eq!(header_pairs(&encrypted), expected(&middle, &signed));
    empty_key_adds_no_header();
}

fn empty_key_adds_no_header() {
    let unencrypted = sign_with(
        Upload {
            content: "",
            key: Some(""),
            ..KMS_UPLOAD
        },
        chain_credentials,
    )
    .unwrap();
    let empty_value = "78e64dd1a40d50176a55a7d8744cad5c2d4f0ae4ffc6fc9e24092033e05a1eeb";
    let empty_auth = signature("S3K/20240101/us-west-2", "", empty_value);
    let empty_signed = [
        ("X-Amz-Content-SHA256", EMPTY_HASH),
        ("Authorization", empty_auth.as_str()),
    ];
    assert_eq!(
        header_pairs(&unencrypted),
        expected(&[("Content-Length", "0")], &empty_signed)
    );
    let without_key = sign_with(
        Upload {
            content: "",
            key: None,
            ..KMS_UPLOAD
        },
        chain_credentials,
    )
    .unwrap();
    assert_eq!(header_pairs(&without_key), header_pairs(&unencrypted));
}

#[test]
fn bedrock_services_s3_signing_credential_identity_isolation() {
    let first = sign(S3_PAIR, "x");
    let other = sign(
        r#"{"s3_access_key_id": "OTHER", "s3_secret_access_key": "S3S"}"#,
        "x",
    );
    let other_value = "3fd0fadbd569f3f19d6c5bfb73401e6bed4c6e8deee2d729731043e73ef9d97f";
    assert_eq!(
        authorization(&other),
        signature("OTHER/20240101/us-west-2", "", other_value)
    );
    assert!(authorization(&first).starts_with("AWS4-HMAC-SHA256 Credential=S3K/"));
    assert_ne!(authorization(&first), authorization(&other));
    // a request with its own aws_* keys leaves nothing behind for the next request without keys
    let session = sign(SESSION, "x");
    assert!(authorization(&session).starts_with("AWS4-HMAC-SHA256 Credential=AKIDSESSION/"));
    let environment = sign("{}", "x");
    let env_prefix = format!("AWS4-HMAC-SHA256 Credential={ENV_ACCESS_KEY}/");
    assert!(authorization(&environment).starts_with(&env_prefix));
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
    s3_pair_skips_the_chain(&first);
}

fn s3_pair_skips_the_chain(first: &AwsSignedRequest) {
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
    let counted = sign_with(upload, counting).unwrap();
    assert_eq!(calls.get(), 0);
    assert_eq!(authorization(&counted), authorization(first));
}

fn secret_failures() -> [S3SigningError; 3] {
    let secret_pair =
        r#"{"s3_access_key_id": "AKIDLEAK", "s3_secret_access_key": "leaked-secret-value"}"#;
    let invalid_url = Upload {
        params: secret_pair,
        content: "x",
        api_base: "not a url",
        key: Some("kms-secret-key"),
    };
    let failure = sign_with(invalid_url, chain_credentials).unwrap_err();
    assert!(matches!(failure, S3SigningError::Sign(_)));
    let failing_chain = |_settings: &AwsSettings, _region: &str| {
        ready(Err::<Credentials, CredentialError>(
            CredentialError::Source(SourceError::MissingProvider),
        ))
    };
    let chain_params =
        r#"{"aws_access_key_id": "AKIDLEAK", "aws_secret_access_key": "leaked-secret-value"}"#;
    let lookup = sign_with(
        Upload {
            params: chain_params,
            ..invalid_url
        },
        failing_chain,
    )
    .unwrap_err();
    assert_eq!(lookup.to_string(), "the AWS credential lookup failed");
    let wrong_params = r#"{"s3_secret_access_key": ["leaked-secret-value"]}"#;
    let wrong_type = sign_with(
        Upload {
            params: wrong_params,
            ..invalid_url
        },
        chain_credentials,
    )
    .unwrap_err();
    assert!(wrong_type.to_string().starts_with("s3_secret_access_key: "));
    [failure, lookup, wrong_type]
}

#[test]
fn bedrock_services_s3_signing_secret_safe_failure() {
    for error in secret_failures() {
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
    let leak_pair =
        r#"{"s3_access_key_id": "AKIDLEAK", "s3_secret_access_key": "leaked-secret-value"}"#;
    let signed = sign(leak_pair, "x");
    let debug = format!("{signed:?}");
    assert!(
        !debug.contains("AKIDLEAK") && !debug.contains("Signature="),
        "{debug}"
    );
}
