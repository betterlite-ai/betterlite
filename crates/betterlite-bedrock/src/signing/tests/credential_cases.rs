use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::mpsc::sync_channel;
use std::task::{Context, Waker};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;
use tokio::runtime::{Builder, Runtime};

use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::signing::body::Body;
use crate::signing::endpoint::{RequestHeaders, request_headers};
use crate::signing::error::SigningError;
use crate::signing::executor::AwsSigningExecutor;
use crate::signing::headers::prepare;
use crate::signing::json_post::{JsonPost, sign_aws_json_post};
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::{RegionInputs, TargetRequest, request_target};

const ENDPOINT: &str =
    "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.claude/invoke";
const NOW_SECONDS: u64 = 1_767_225_600;
const SECRET_KEY: &str = "fixture-secret-access-key";
const SECRET_TOKEN: &str = "fixture-session-token";

fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(NOW_SECONDS))
        .unwrap()
}

fn test_fixture_credentials(access: &str, secret: &str, token: Option<&str>) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(access.to_owned()),
        secret_access_key: SecretString::from(secret.to_owned()),
        session_token: token.map(|session| SecretString::from(session.to_owned())),
    }
}

fn test_base_headers() -> Vec<(String, Option<String>)> {
    vec![
        (
            "content-type".to_owned(),
            Some("application/json".to_owned()),
        ),
        ("x-custom".to_owned(), Some("forwarded".to_owned())),
    ]
}

fn test_signed_with(
    credentials: &Credentials,
    headers: &[(String, Option<String>)],
) -> AwsSignedRequest {
    request_headers(RequestHeaders {
        credentials: Some(credentials),
        region: "us-west-2",
        extra_headers: &[],
        endpoint: ENDPOINT,
        data: Body::Text(r#"{"inputText":"hello"}"#),
        headers,
        api_key: None,
        configured_bearer: None,
        supports_bearer_token: true,
        now: now(),
    })
    .unwrap()
}

fn test_header(signed: &AwsSignedRequest, name: &str) -> Option<String> {
    signed
        .headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.clone())
}

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

fn test_identity_signatures() {
    let alice = test_fixture_credentials("AKIDALICE", "alice-secret", Some("alice-token"));
    let bob = test_fixture_credentials("AKIDBOB", "bob-secret", Some("bob-token"));
    let headers = test_base_headers();
    let alice_signed = test_signed_with(&alice, &headers);
    let bob_signed = test_signed_with(&bob, &headers);
    assert!(
        test_header(&alice_signed, "Authorization")
            .unwrap()
            .contains("Credential=AKIDALICE/")
    );
    assert!(
        test_header(&bob_signed, "Authorization")
            .unwrap()
            .contains("Credential=AKIDBOB/")
    );
    assert_eq!(
        test_header(&alice_signed, "X-Amz-Security-Token").as_deref(),
        Some("alice-token")
    );
    assert_eq!(
        test_header(&bob_signed, "X-Amz-Security-Token").as_deref(),
        Some("bob-token")
    );
    assert_ne!(
        test_header(&alice_signed, "Authorization"),
        test_header(&bob_signed, "Authorization")
    );
}

fn test_stale_token_is_replaced() {
    let alice = test_fixture_credentials("AKIDALICE", "alice-secret", Some("alice-token"));
    let stale = vec![
        (
            "content-type".to_owned(),
            Some("application/json".to_owned()),
        ),
        (
            "x-amz-security-token".to_owned(),
            Some("bob-token".to_owned()),
        ),
    ];
    let refreshed = test_signed_with(&alice, &stale);
    assert_eq!(
        test_header(&refreshed, "X-Amz-Security-Token").as_deref(),
        Some("alice-token")
    );
}

async fn test_concurrent_signatures() {
    let alice = test_fixture_credentials("AKIDALICE", "alice-secret", Some("alice-token"));
    let bob = test_fixture_credentials("AKIDBOB", "bob-secret", Some("bob-token"));
    let executor = AwsSigningExecutor::default();
    let alice_headers = test_base_headers();
    let bob_headers = test_base_headers();
    let (first_result, second_result) = tokio::join!(
        executor.run(move || test_signed_with(&alice, &alice_headers)),
        executor.run(move || test_signed_with(&bob, &bob_headers))
    );
    let first = first_result.unwrap();
    let second = second_result.unwrap();
    assert!(
        test_header(&first, "Authorization")
            .unwrap()
            .contains("Credential=AKIDALICE/")
    );
    assert!(
        test_header(&second, "Authorization")
            .unwrap()
            .contains("Credential=AKIDBOB/")
    );
}

fn test_cancelled_signature() {
    let alice = test_fixture_credentials("AKIDALICE", "alice-secret", Some("alice-token"));
    let bob = test_fixture_credentials("AKIDBOB", "bob-secret", Some("bob-token"));
    let executor = AwsSigningExecutor::default();
    let bob_headers = test_base_headers();
    let (release, gate) = sync_channel::<()>(1);
    let mut cancelled: Pin<
        Box<dyn Future<Output = Result<AwsSignedRequest, SigningError>> + Send>,
    > = Box::pin(executor.run(move || {
        gate.recv().unwrap();
        test_signed_with(&bob, &bob_headers)
    }));
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    assert!(cancelled.as_mut().poll(&mut context).is_pending());
    drop(cancelled);
    // the aborted task may never have started, which drops the receiver
    release.send(()).unwrap_or_default();
    let after = test_signed_with(&alice, &test_base_headers());
    assert_eq!(
        test_header(&after, "X-Amz-Security-Token").as_deref(),
        Some("alice-token")
    );
}

#[tokio::test]
async fn bedrock_signing_credential_identity_isolation() {
    test_identity_signatures();
    test_stale_token_is_replaced();
    test_concurrent_signatures().await;
    test_cancelled_signature();
}

fn test_json_post_authorization(extra: &[(String, String)]) -> String {
    let credentials = test_fixture_credentials("AKIDEXAMPLE", "fixture-secret", None);
    let mut headers = vec![("content-type".to_owned(), "application/json".to_owned())];
    headers.extend_from_slice(extra);
    let signed = sign_aws_json_post(JsonPost {
        service_name: "bedrock",
        region: Some("us-west-2"),
        url: ENDPOINT,
        body: "{}",
        headers: &headers,
        credentials: &credentials,
        now: now(),
    })
    .unwrap();
    test_header(&signed, "Authorization").unwrap()
}

fn test_credential_settings() {
    let mut params: BTreeMap<String, Box<RawValue>> = serde_json::from_str(
        r#"{"aws_region_name":"us-west-2","aws_access_key_id":"AKIDEXAMPLE"}"#,
    )
    .unwrap();
    let target = runtime()
        .block_on(request_target(
            TargetRequest {
                optional_params: &mut params,
                model: None,
                region_inputs: RegionInputs::default(),
                bearer_token: None,
            },
            |settings, region| {
                let key = settings.access_key_id.clone().unwrap();
                let echoed = format!("{:?}/{region}", settings.region_name);
                ready(Ok::<Credentials, CredentialError>(Credentials {
                    access_key_id: key,
                    secret_access_key: SecretString::from(echoed),
                    session_token: None,
                }))
            },
        ))
        .unwrap();
    let credentials = target.credentials.unwrap();
    assert_eq!(credentials.access_key_id.expose_secret(), "AKIDEXAMPLE");
    assert_eq!(
        credentials.secret_access_key.expose_secret(),
        "Some(\"us-west-2\")/us-west-2"
    );
}

#[test]
fn bedrock_signing_unsigned_headers_and_settings() {
    let plain = test_json_post_authorization(&[]);
    let unsigned = [
        ("connection".to_owned(), "keep-alive".to_owned()),
        ("te".to_owned(), "trailers".to_owned()),
        ("upgrade".to_owned(), "h2c".to_owned()),
    ];
    assert_eq!(test_json_post_authorization(&unsigned), plain);
    let chunked = vec![("Transfer-Encoding".to_owned(), "chunked".to_owned())];
    assert_eq!(prepare(chunked.clone(), 5, "POST"), chunked);
    test_credential_settings();
}

fn test_no_credentials_failure() -> SigningError {
    request_headers(RequestHeaders {
        credentials: None,
        region: "us-west-2",
        extra_headers: &[],
        endpoint: ENDPOINT,
        data: Body::Text("{}"),
        headers: &test_base_headers(),
        api_key: None,
        configured_bearer: None,
        supports_bearer_token: true,
        now: now(),
    })
    .unwrap_err()
}

fn test_bad_endpoint_failure(credentials: &Credentials) -> SigningError {
    request_headers(RequestHeaders {
        credentials: Some(credentials),
        region: "us-west-2",
        extra_headers: &[],
        endpoint: &format!("{SECRET_KEY}\n"),
        data: Body::Text("{}"),
        headers: &test_base_headers(),
        api_key: None,
        configured_bearer: None,
        supports_bearer_token: true,
        now: now(),
    })
    .unwrap_err()
}

fn test_missing_region_failure(credentials: &Credentials) -> SigningError {
    sign_aws_json_post(JsonPost {
        service_name: "bedrock",
        region: None,
        url: ENDPOINT,
        body: "{}",
        headers: &[],
        credentials,
        now: now(),
    })
    .unwrap_err()
}

fn test_request_failures(credentials: &Credentials) -> Vec<SigningError> {
    vec![
        test_no_credentials_failure(),
        test_bad_endpoint_failure(credentials),
        test_missing_region_failure(credentials),
    ]
}

fn test_target_failure(credentials: &Credentials) -> SigningError {
    let mut params: BTreeMap<String, Box<RawValue>> = serde_json::from_str(
        r#"{"aws_region_name":"us-west-2","aws_access_key_id":{"fixture-secret-access-key":true}}"#,
    )
    .unwrap();
    let result = runtime().block_on(request_target(
        TargetRequest {
            optional_params: &mut params,
            model: None,
            region_inputs: RegionInputs::default(),
            bearer_token: None,
        },
        |_settings, _region| ready(Ok::<Credentials, CredentialError>(credentials.clone())),
    ));
    result.unwrap_err()
}

fn test_assert_no_secret(error: &SigningError) {
    let text = format!("{error} {error:?}");
    assert!(!text.contains(SECRET_KEY));
    assert!(!text.contains(SECRET_TOKEN));
}

#[test]
fn bedrock_signing_secret_safe_failure() {
    let credentials = test_fixture_credentials("AKIDFIXTURE", SECRET_KEY, Some(SECRET_TOKEN));
    let failures = test_request_failures(&credentials);
    assert!(failures.iter().all(|error| {
        let text = format!("{error} {error:?}");
        !text.contains(SECRET_KEY) && !text.contains(SECRET_TOKEN)
    }));
    let target = test_target_failure(&credentials);
    test_assert_no_secret(&target);
    assert!(matches!(target, SigningError::Auth(_)));
    let credential_debug = format!("{credentials:?}");
    assert!(!credential_debug.contains(SECRET_KEY));
    assert!(!credential_debug.contains(SECRET_TOKEN));
    assert_eq!(credentials.secret_access_key.expose_secret(), SECRET_KEY);
    let signed = test_signed_with(&credentials, &test_base_headers());
    let signed_debug = format!("{signed:?}");
    assert!(!signed_debug.contains(SECRET_TOKEN));
    assert!(signed_debug.contains("header_names"));
    assert_eq!(failures[0].to_string(), "Unable to locate credentials");
}
