use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
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

fn fixture_credentials(access: &str, secret: &str, token: Option<&str>) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(access.to_owned()),
        secret_access_key: SecretString::from(secret.to_owned()),
        session_token: token.map(|session| SecretString::from(session.to_owned())),
    }
}

fn base_headers() -> Vec<(String, Option<String>)> {
    vec![
        (
            "content-type".to_owned(),
            Some("application/json".to_owned()),
        ),
        ("x-custom".to_owned(), Some("forwarded".to_owned())),
    ]
}

fn signed_with(
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

fn header(signed: &AwsSignedRequest, name: &str) -> Option<String> {
    signed
        .headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.clone())
}

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

fn identity_signatures() {
    let alice = fixture_credentials("AKIDALICE", "alice-secret", Some("alice-token"));
    let bob = fixture_credentials("AKIDBOB", "bob-secret", Some("bob-token"));
    let headers = base_headers();
    let alice_signed = signed_with(&alice, &headers);
    let bob_signed = signed_with(&bob, &headers);
    assert!(
        header(&alice_signed, "Authorization")
            .unwrap()
            .contains("Credential=AKIDALICE/")
    );
    assert!(
        header(&bob_signed, "Authorization")
            .unwrap()
            .contains("Credential=AKIDBOB/")
    );
    assert_eq!(
        header(&alice_signed, "X-Amz-Security-Token").as_deref(),
        Some("alice-token")
    );
    assert_eq!(
        header(&bob_signed, "X-Amz-Security-Token").as_deref(),
        Some("bob-token")
    );
    assert_ne!(
        header(&alice_signed, "Authorization"),
        header(&bob_signed, "Authorization")
    );
}

fn stale_token_is_replaced() {
    let alice = fixture_credentials("AKIDALICE", "alice-secret", Some("alice-token"));
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
    let refreshed = signed_with(&alice, &stale);
    assert_eq!(
        header(&refreshed, "X-Amz-Security-Token").as_deref(),
        Some("alice-token")
    );
}

async fn concurrent_signatures() {
    let alice = fixture_credentials("AKIDALICE", "alice-secret", Some("alice-token"));
    let bob = fixture_credentials("AKIDBOB", "bob-secret", Some("bob-token"));
    let executor = AwsSigningExecutor::default();
    let alice_headers = base_headers();
    let bob_headers = base_headers();
    let (first_result, second_result) = tokio::join!(
        executor.run(move || signed_with(&alice, &alice_headers)),
        executor.run(move || signed_with(&bob, &bob_headers))
    );
    let first = first_result.unwrap();
    let second = second_result.unwrap();
    assert!(
        header(&first, "Authorization")
            .unwrap()
            .contains("Credential=AKIDALICE/")
    );
    assert!(
        header(&second, "Authorization")
            .unwrap()
            .contains("Credential=AKIDBOB/")
    );
}

fn cancelled_signature_keeps_identity() {
    let alice = fixture_credentials("AKIDALICE", "alice-secret", Some("alice-token"));
    let bob = fixture_credentials("AKIDBOB", "bob-secret", Some("bob-token"));
    let executor = AwsSigningExecutor::default();
    let bob_headers = base_headers();
    let mut cancelled: Pin<
        Box<dyn Future<Output = Result<AwsSignedRequest, SigningError>> + Send>,
    > = Box::pin(executor.run(move || signed_with(&bob, &bob_headers)));
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    assert!(cancelled.as_mut().poll(&mut context).is_pending());
    drop(cancelled);
    let after = signed_with(&alice, &base_headers());
    assert_eq!(
        header(&after, "X-Amz-Security-Token").as_deref(),
        Some("alice-token")
    );
}

#[tokio::test]
async fn bedrock_signing_credential_identity_isolation() {
    identity_signatures();
    stale_token_is_replaced();
    concurrent_signatures().await;
    cancelled_signature_keeps_identity();
}

fn no_credentials_failure() -> SigningError {
    request_headers(RequestHeaders {
        credentials: None,
        region: "us-west-2",
        extra_headers: &[],
        endpoint: ENDPOINT,
        data: Body::Text("{}"),
        headers: &base_headers(),
        api_key: None,
        configured_bearer: None,
        supports_bearer_token: true,
        now: now(),
    })
    .unwrap_err()
}

fn bad_region_failure(credentials: &Credentials) -> SigningError {
    request_headers(RequestHeaders {
        credentials: Some(credentials),
        region: "NOT-A-REGION",
        extra_headers: &[],
        endpoint: ENDPOINT,
        data: Body::Text("{}"),
        headers: &base_headers(),
        api_key: None,
        configured_bearer: None,
        supports_bearer_token: true,
        now: now(),
    })
    .unwrap_err()
}

fn bad_header_failure(credentials: &Credentials) -> SigningError {
    let bad_header = vec![("x-bad".to_owned(), Some(format!("{SECRET_KEY}\n")))];
    request_headers(RequestHeaders {
        credentials: Some(credentials),
        region: "us-west-2",
        extra_headers: &[],
        endpoint: ENDPOINT,
        data: Body::Text("{}"),
        headers: &bad_header,
        api_key: None,
        configured_bearer: None,
        supports_bearer_token: true,
        now: now(),
    })
    .unwrap_err()
}

fn missing_region_failure(credentials: &Credentials) -> SigningError {
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

fn request_failures(credentials: &Credentials) -> Vec<SigningError> {
    vec![
        no_credentials_failure(),
        bad_region_failure(credentials),
        bad_header_failure(credentials),
        missing_region_failure(credentials),
    ]
}

fn target_failure(credentials: &Credentials) -> SigningError {
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

fn assert_no_secret(error: &SigningError) {
    let text = format!("{error} {error:?}");
    assert!(!text.contains(SECRET_KEY));
    assert!(!text.contains(SECRET_TOKEN));
}

#[test]
fn bedrock_signing_secret_safe_failure() {
    let credentials = fixture_credentials("AKIDFIXTURE", SECRET_KEY, Some(SECRET_TOKEN));
    let failures = request_failures(&credentials);
    assert!(failures.iter().all(|error| {
        let text = format!("{error} {error:?}");
        !text.contains(SECRET_KEY) && !text.contains(SECRET_TOKEN)
    }));
    let target = target_failure(&credentials);
    assert_no_secret(&target);
    assert!(matches!(target, SigningError::Auth(_)));
    let credential_debug = format!("{credentials:?}");
    assert!(!credential_debug.contains(SECRET_KEY));
    assert!(!credential_debug.contains(SECRET_TOKEN));
    assert_eq!(credentials.secret_access_key.expose_secret(), SECRET_KEY);
    let signed = signed_with(&credentials, &base_headers());
    let signed_debug = format!("{signed:?}");
    assert!(!signed_debug.contains(SECRET_TOKEN));
    assert!(signed_debug.contains("<redacted>"));
    assert_eq!(failures[0].to_string(), "Unable to locate credentials");
}
