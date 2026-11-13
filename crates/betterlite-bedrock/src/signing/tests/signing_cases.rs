use std::cell::Cell;
use std::collections::BTreeMap;
use std::future::ready;
use std::thread::current;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;
use tokio::runtime::{Builder, Runtime};
use tracing::subscriber::set_default;
use tracing_subscriber::registry;

use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::signing::batch::{BatchData, BatchRequest, sign_batch_request};
use crate::signing::body::Body;
use crate::signing::endpoint::{RequestHeaders, request_headers};
use crate::signing::error::SigningError;
use crate::signing::executor::{AwsSigningExecutor, sign_request_off_loop_if_aws};
use crate::signing::headers::filter_for_signature;
use crate::signing::json_post::{JsonPost, sign_aws_json_post};
use crate::signing::request::AwsSignedRequest;
use crate::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use crate::signing::target::{RegionInputs, RequestTarget, TargetRequest, request_target};

const SECRET: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
const ENDPOINT: &str =
    "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.claude/invoke";
const NOW_SECONDS: u64 = 1_767_225_600;
const REGION_PARAMS: &str = r#"{"aws_access_key_id":"AKIDEXAMPLE","aws_secret_access_key":"wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY","aws_region_name":"us-west-2"}"#;
const TOKEN_PARAMS: &str = r#"{"aws_access_key_id":"AKIDEXAMPLE","aws_secret_access_key":"wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY","aws_session_token":"session-token","aws_region_name":"us-west-2"}"#;
const AUTH_FIXED: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=f9f1d0467f690e2ccc70a909cd81c1f084129488ec8435fd24bb9f45db51dedf";
const AUTH_TOKEN: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token, Signature=639da7857892a555ed9d822e08e6d38ad1f4f5cd98d4c43563b46a3d78466a96";
const AUTH_PLAIN: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=b764453773c9a020590680c92d2d9271c13a956c1764a02eb90fddf867a22f7b";
const AUTH_HELLO: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=2cbe2522f38ee1d2ae76e0a4734f6839675bdc815891971d03a7909aee312831";
const AUTH_SERVICE: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token, Signature=e21512313a6fae7eba4d293462dba2c2ac7071652a5079bf24b89607f8d3ac64";
const AUTH_SAGEMAKER: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/sagemaker/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=c5ac597fdbca14b17ea7abd611fe8837ae775697d17e06979d0f3f5a0f250d76";
const AUTH_S3VECTORS: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/s3vectors/aws4_request, SignedHeaders=host;x-amz-date, Signature=945800bf5c4bbdc83f0ee9b5cc1da00bee9e83dbbd55a49e28a1bdb2c0b941c1";
const AUTH_JSON_POST: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-custom, Signature=3b5318571b79d09d2d2700e3206fbe8589965c00188bf2e4afd2c0a867201f5e";
const AUTH_AWS4_OVERRIDE: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=b94efd53a456048c5951d9827e615bd32dae3edafba27389f5e5f7b8744cdaea";
const AUTH_BATCH_GET: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/bedrock/aws4_request, SignedHeaders=host;x-amz-date, Signature=264c87e326d2c8750f940f74916aee6a31c25add59a756020eb690a9bc1f8f0a";
const AUTH_BATCH_POST: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=a79c6c5b68cf97e97203c8fdff374d6d0b3d6703006209f0536e1211004ef4bf";
const AUTH_BATCH_S3: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/s3/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token, Signature=431103f0957df0a9813ade8c3e8d49414f172273b785f5f9d7fc0b3a81a69a59";

fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(NOW_SECONDS))
        .unwrap()
}

fn fixture_credentials(access: &str, token: Option<&str>) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(access.to_owned()),
        secret_access_key: SecretString::from(SECRET.to_owned()),
        session_token: token.map(|session| SecretString::from(session.to_owned())),
    }
}

fn supplied(pairs: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.map(str::to_owned)))
        .collect()
}

fn expected(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn header_map(headers: &[(String, String)]) -> BTreeMap<String, String> {
    headers.iter().cloned().collect()
}

fn optional_params(source: &str) -> BTreeMap<String, Box<RawValue>> {
    serde_json::from_str(source).unwrap()
}

fn request_data(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).unwrap()
}

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

fn test_assert_signed(signed: &AwsSignedRequest, url: &str, body: &[u8], headers: &[(&str, &str)]) {
    assert_eq!(signed.url, url);
    assert_eq!(signed.body, body.to_vec());
    assert_eq!(header_map(&signed.headers), expected(headers));
}

#[derive(Clone, Copy)]
struct EndpointCase<'source> {
    credentials: Option<&'source Credentials>,
    headers: &'source [(String, Option<String>)],
    extra: &'source [(String, String)],
    data: Body<'source>,
    api_key: Option<&'source SecretString>,
    configured: Option<&'source SecretString>,
    supports: bool,
}

fn test_endpoint(case: EndpointCase<'_>) -> Result<AwsSignedRequest, SigningError> {
    request_headers(RequestHeaders {
        credentials: case.credentials,
        region: "us-west-2",
        extra_headers: case.extra,
        endpoint: ENDPOINT,
        data: case.data,
        headers: case.headers,
        api_key: case.api_key,
        configured_bearer: case.configured,
        supports_bearer_token: case.supports,
        now: now(),
    })
}

fn test_plain_endpoint(
    credentials: &Credentials,
    headers: &[(String, Option<String>)],
    extra: &[(String, String)],
    data: Body<'_>,
) -> AwsSignedRequest {
    test_endpoint(EndpointCase {
        credentials: Some(credentials),
        headers,
        extra,
        data,
        api_key: None,
        configured: None,
        supports: true,
    })
    .unwrap()
}

#[derive(Clone, Copy)]
struct ServiceCase<'source> {
    service: ServiceName,
    headers: &'source [(String, Option<String>)],
    params: &'source str,
    data: &'source str,
    api_base: &'source str,
    fixture: &'source Credentials,
    api_key: Option<&'source SecretString>,
    configured: Option<&'source SecretString>,
}

fn test_service(case: ServiceCase<'_>) -> Result<AwsSignedRequest, SigningError> {
    let params = optional_params(case.params);
    let data = request_data(case.data);
    runtime().block_on(sign_service_request(
        ServiceRequest {
            service: case.service,
            headers: case.headers,
            optional_params: &params,
            request_data: &data,
            api_base: case.api_base,
            model: None,
            api_key: case.api_key,
            configured_bearer: case.configured,
            region_inputs: RegionInputs::default(),
            now: now(),
        },
        |_settings, _region| ready(Ok::<Credentials, CredentialError>(case.fixture.clone())),
    ))
}

#[derive(Clone, Copy)]
struct BatchCase<'source> {
    service_name: &'source str,
    data: BatchData<'source>,
    endpoint: &'source str,
    params: &'source str,
    method: &'source str,
    fixture: &'source Credentials,
}

fn test_batch(case: BatchCase<'_>) -> Result<AwsSignedRequest, SigningError> {
    let params = optional_params(case.params);
    runtime().block_on(sign_batch_request(
        BatchRequest {
            service_name: case.service_name,
            data: case.data,
            endpoint: case.endpoint,
            optional_params: &params,
            method: case.method,
            region_inputs: RegionInputs::default(),
            now: now(),
        },
        |_settings, _region| ready(Ok::<Credentials, CredentialError>(case.fixture.clone())),
    ))
}

fn fixed_clock_sigv4() {
    let credentials = fixture_credentials("AKIDEXAMPLE", None);
    let headers = supplied(&[
        ("content-type", Some("application/json")),
        ("x-custom", Some("keep")),
    ]);
    let signed = test_plain_endpoint(
        &credentials,
        &headers,
        &[],
        Body::Text(r#"{"messages":[{"role":"user","content":"hello"}]}"#),
    );
    assert_eq!(signed.method, "POST");
    test_assert_signed(
        &signed,
        ENDPOINT,
        br#"{"messages":[{"role":"user","content":"hello"}]}"#,
        &[
            ("Authorization", AUTH_FIXED),
            ("Content-Length", "48"),
            ("X-Amz-Date", "20260101T000000Z"),
            ("content-type", "application/json"),
            ("x-custom", "keep"),
        ],
    );
}

fn session_token_headers() {
    let with_token = fixture_credentials("AKIDEXAMPLE", Some("session-token"));
    let headers = supplied(&[("content-type", Some("application/json"))]);
    let signed = test_plain_endpoint(
        &with_token,
        &headers,
        &[],
        Body::Text(r#"{"inputText":"hello"}"#),
    );
    test_assert_signed(
        &signed,
        ENDPOINT,
        br#"{"inputText":"hello"}"#,
        &[
            ("Authorization", AUTH_TOKEN),
            ("Content-Length", "21"),
            ("X-Amz-Date", "20260101T000000Z"),
            ("X-Amz-Security-Token", "session-token"),
            ("content-type", "application/json"),
        ],
    );
}

fn plain_signature_headers() {
    let without_token = fixture_credentials("AKIDEXAMPLE", None);
    let headers = supplied(&[("content-type", Some("application/json"))]);
    let signed = test_plain_endpoint(
        &without_token,
        &headers,
        &[],
        Body::Text(r#"{"inputText":"hello"}"#),
    );
    test_assert_signed(
        &signed,
        ENDPOINT,
        br#"{"inputText":"hello"}"#,
        &[
            ("Authorization", AUTH_PLAIN),
            ("Content-Length", "21"),
            ("X-Amz-Date", "20260101T000000Z"),
            ("content-type", "application/json"),
        ],
    );
}

fn session_token_signature() {
    session_token_headers();
    plain_signature_headers();
}

fn allowlist_headers() -> BTreeMap<String, String> {
    expected(&[
        ("host", "bedrock-runtime.us-west-2.amazonaws.com"),
        ("Content-Type", "application/json"),
        ("Date", "Thu, 01 Jan 2026 00:00:00 GMT"),
        ("X-Amz-Date", "20260101T000000Z"),
        ("x-amz-security-token", "session-token"),
        ("X-Amz-Content-SHA256", "abc"),
        ("x-amz-algorithm", "AWS4-HMAC-SHA256"),
        (
            "x-amz-credential",
            "AKIDEXAMPLE/20260101/us-west-2/bedrock/aws4_request",
        ),
        ("x-amz-signedheaders", "host"),
        ("x-amz-signature", "deadbeef"),
        ("x-amzn-trace-id", "Root=1-abc"),
    ])
}

fn signed_header_allowlist() {
    let headers = supplied(&[
        ("host", Some("bedrock-runtime.us-west-2.amazonaws.com")),
        ("Content-Type", Some("application/json")),
        ("Date", Some("Thu, 01 Jan 2026 00:00:00 GMT")),
        ("X-Amz-Date", Some("20260101T000000Z")),
        ("x-amz-security-token", Some("session-token")),
        ("X-Amz-Content-SHA256", Some("abc")),
        ("x-amz-algorithm", Some("AWS4-HMAC-SHA256")),
        (
            "x-amz-credential",
            Some("AKIDEXAMPLE/20260101/us-west-2/bedrock/aws4_request"),
        ),
        ("x-amz-signedheaders", Some("host")),
        ("x-amz-signature", Some("deadbeef")),
        ("x-amzn-trace-id", Some("Root=1-abc")),
        ("x-custom", Some("keep")),
        ("user-agent", Some("fixture")),
        ("connection", Some("keep-alive")),
        ("x-null", None),
    ]);
    assert_eq!(
        header_map(&filter_for_signature(&headers)),
        allowlist_headers()
    );
}

fn computed_header_replacement() {
    let credentials = fixture_credentials("AKIDEXAMPLE", Some("session-token"));
    let stale = supplied(&[
        ("authorization", Some("stale-authorization")),
        ("x-amz-date", Some("20200101T000000Z")),
        ("x-amz-security-token", Some("stale-token")),
        ("content-type", Some("application/json")),
        ("x-custom", Some("forwarded")),
        ("x-forwarded-for", Some("203.0.113.7")),
    ]);
    let forwarded = supplied(&[
        ("content-type", Some("application/json")),
        ("x-custom", Some("forwarded")),
        ("x-forwarded-for", Some("203.0.113.7")),
    ]);
    let body = br#"{"inputText":"hello"}"#;
    let expected_headers = [
        ("Authorization", AUTH_TOKEN),
        ("Content-Length", "21"),
        ("X-Amz-Date", "20260101T000000Z"),
        ("X-Amz-Security-Token", "session-token"),
        ("content-type", "application/json"),
        ("x-custom", "forwarded"),
        ("x-forwarded-for", "203.0.113.7"),
    ];
    let with_stale = test_plain_endpoint(
        &credentials,
        &stale,
        &[],
        Body::Text(r#"{"inputText":"hello"}"#),
    );
    let without_stale = test_plain_endpoint(
        &credentials,
        &forwarded,
        &[],
        Body::Text(r#"{"inputText":"hello"}"#),
    );
    test_assert_signed(&with_stale, ENDPOINT, body, &expected_headers);
    test_assert_signed(&without_stale, ENDPOINT, body, &expected_headers);
}

fn bearer_request_headers(api_key: &SecretString) -> AwsSignedRequest {
    let headers = supplied(&[
        ("content-type", Some("application/json")),
        ("x-custom", Some("keep")),
    ]);
    let extra = [("Authorization".to_owned(), "ignored-in-bearer".to_owned())];
    test_endpoint(EndpointCase {
        credentials: None,
        headers: &headers,
        extra: &extra,
        data: Body::Text(r#"{"inputText":"hello"}"#),
        api_key: Some(api_key),
        configured: None,
        supports: true,
    })
    .unwrap()
}

fn bearer_service_request(api_key: &SecretString) -> (AwsSignedRequest, usize) {
    let calls = Cell::new(0_usize);
    let fixture = fixture_credentials("UNUSED", None);
    let headers = supplied(&[("content-type", Some("text/plain"))]);
    let params = optional_params(r#"{"aws_access_key_id":{"bad":1}}"#);
    let data = request_data("{\"b\":1,\"a\":\"h\u{e9}llo\",\"n\":1.0}");
    let calls_ref = &calls;
    let signed = runtime().block_on(sign_service_request(
        ServiceRequest {
            service: ServiceName::BedrockAgentcore,
            headers: &headers,
            optional_params: &params,
            request_data: &data,
            api_base: "https://bedrock-agentcore.us-west-2.amazonaws.com/runtimes/x/invocations",
            model: None,
            api_key: Some(api_key),
            configured_bearer: None,
            region_inputs: RegionInputs::default(),
            now: now(),
        },
        |_settings, _region| {
            calls_ref.set(calls_ref.get().saturating_add(1));
            ready(Ok::<Credentials, CredentialError>(fixture.clone()))
        },
    ))
    .unwrap();
    (signed, calls.get())
}

fn bearer_skips_sts() {
    let api_key = SecretString::from("bedrock-api-key".to_owned());
    let signed = bearer_request_headers(&api_key);
    test_assert_signed(
        &signed,
        ENDPOINT,
        br#"{"inputText":"hello"}"#,
        &[
            ("Authorization", "Bearer bedrock-api-key"),
            ("Content-Length", "21"),
            ("content-type", "application/json"),
            ("x-custom", "keep"),
        ],
    );
    let (service_signed, calls) = bearer_service_request(&api_key);
    assert_eq!(calls, 0);
    test_assert_signed(
        &service_signed,
        "https://bedrock-agentcore.us-west-2.amazonaws.com/runtimes/x/invocations",
        b"{\"b\": 1, \"a\": \"h\\u00e9llo\", \"n\": 1.0}",
        &[
            ("Authorization", "Bearer bedrock-api-key"),
            ("Content-Type", "application/json"),
            ("content-type", "text/plain"),
        ],
    );
    let configured = SecretString::from("env-token".to_owned());
    let headers = supplied(&[("content-type", Some("application/json"))]);
    let missing = test_endpoint(EndpointCase {
        credentials: None,
        headers: &headers,
        extra: &[],
        data: Body::Text("{}"),
        api_key: Some(&api_key),
        configured: Some(&configured),
        supports: false,
    })
    .unwrap_err();
    assert!(matches!(missing, SigningError::NoCredentials));
}

fn override_request_headers(extra: &[(String, String)]) -> AwsSignedRequest {
    let credentials = fixture_credentials("AKIDEXAMPLE", None);
    let headers = supplied(&[("content-type", Some("application/json"))]);
    test_plain_endpoint(
        &credentials,
        &headers,
        extra,
        Body::Text(r#"{"inputText":"hello"}"#),
    )
}

fn override_service(authorization: Option<&str>) -> BTreeMap<String, String> {
    let credentials = fixture_credentials("AKIDEXAMPLE", None);
    let headers = supplied(&[
        ("content-type", Some("application/json")),
        ("authorization", authorization),
    ]);
    let signed = test_service(ServiceCase {
        service: ServiceName::Bedrock,
        headers: &headers,
        params: REGION_PARAMS,
        data: r#"{"a":1}"#,
        api_base: ENDPOINT,
        fixture: &credentials,
        api_key: None,
        configured: None,
    })
    .unwrap();
    header_map(&signed.headers)
}

fn test_assert_service_override(authorization: Option<&str>, expected_authorization: &str) {
    let headers = override_service(authorization);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some(expected_authorization)
    );
    assert_eq!(headers.len(), 3);
}

fn override_request_checks() {
    let extra_override = [
        ("Authorization".to_owned(), "custom-auth".to_owned()),
        ("authorization".to_owned(), "lower-ignored".to_owned()),
    ];
    let overridden = override_request_headers(&extra_override);
    test_assert_signed(
        &overridden,
        ENDPOINT,
        br#"{"inputText":"hello"}"#,
        &[
            ("Authorization", "custom-auth"),
            ("Content-Length", "21"),
            ("X-Amz-Date", "20260101T000000Z"),
            ("content-type", "application/json"),
        ],
    );
    let aws_extra = [(
        "Authorization".to_owned(),
        "AWS4-HMAC-SHA256 manual".to_owned(),
    )];
    let kept = override_request_headers(&aws_extra);
    test_assert_signed(
        &kept,
        ENDPOINT,
        br#"{"inputText":"hello"}"#,
        &[
            ("Authorization", AUTH_PLAIN),
            ("Content-Length", "21"),
            ("X-Amz-Date", "20260101T000000Z"),
            ("content-type", "application/json"),
        ],
    );
}

fn explicit_non_aws_auth_override() {
    override_request_checks();
    test_assert_service_override(Some("custom-lower"), "custom-lower");
    test_assert_service_override(Some("AWS4-HMAC-SHA256 manual"), AUTH_AWS4_OVERRIDE);
    test_assert_service_override(Some(""), "");
    test_assert_service_override(Some("aws4-hmac-sha256 manual"), "aws4-hmac-sha256 manual");
}

fn empty_credentials_error() {
    let headers = supplied(&[("content-type", Some("application/json"))]);
    let api_key = SecretString::from("bedrock-api-key".to_owned());
    let plain = test_endpoint(EndpointCase {
        credentials: None,
        headers: &headers,
        extra: &[],
        data: Body::Text("{}"),
        api_key: None,
        configured: None,
        supports: false,
    })
    .unwrap_err();
    let bearer_disabled = test_endpoint(EndpointCase {
        credentials: None,
        headers: &headers,
        extra: &[],
        data: Body::Text("{}"),
        api_key: Some(&api_key),
        configured: None,
        supports: false,
    })
    .unwrap_err();
    assert_eq!(plain.to_string(), "Unable to locate credentials");
    assert_eq!(bearer_disabled.to_string(), "Unable to locate credentials");
}

fn assert_text_and_bytes_agree() {
    let credentials = fixture_credentials("AKIDEXAMPLE", None);
    let headers = supplied(&[("content-type", Some("application/json"))]);
    let text = test_plain_endpoint(
        &credentials,
        &headers,
        &[],
        Body::Text("{\"text\":\"h\u{e9}llo\"}"),
    );
    let bytes = test_plain_endpoint(
        &credentials,
        &headers,
        &[],
        Body::Bytes("{\"text\":\"h\u{e9}llo\"}".as_bytes()),
    );
    assert_eq!(text.body, bytes.body);
    let expected_text = [
        ("Authorization", AUTH_HELLO),
        ("Content-Length", "16"),
        ("X-Amz-Date", "20260101T000000Z"),
        ("content-type", "application/json"),
    ];
    assert_eq!(header_map(&text.headers), expected(&expected_text));
    let expected_bytes = [
        ("Authorization", AUTH_HELLO),
        ("Content-Length", "17"),
        ("X-Amz-Date", "20260101T000000Z"),
        ("content-type", "application/json"),
    ];
    assert_eq!(header_map(&bytes.headers), expected(&expected_bytes));
}

fn bedrock_body_signature() {
    let fixture = fixture_credentials("AKIDEXAMPLE", Some("session-token"));
    let empty = Vec::new();
    let signed = test_service(ServiceCase {
        service: ServiceName::Bedrock,
        headers: &empty,
        params: TOKEN_PARAMS,
        data: r#"{"b":1,"a":2}"#,
        api_base: ENDPOINT,
        fixture: &fixture,
        api_key: None,
        configured: None,
    })
    .unwrap();
    assert_eq!(signed.method, "POST");
    test_assert_signed(
        &signed,
        ENDPOINT,
        br#"{"b": 1, "a": 2}"#,
        &[
            ("Authorization", AUTH_SERVICE),
            ("Content-Type", "application/json"),
            ("X-Amz-Date", "20260101T000000Z"),
            ("X-Amz-Security-Token", "session-token"),
        ],
    );
}

fn sagemaker_keeps_content_type() {
    let fixture = fixture_credentials("AKIDEXAMPLE", None);
    let custom = supplied(&[("content-type", Some("application/x-custom"))]);
    let signed = test_service(ServiceCase {
        service: ServiceName::Sagemaker,
        headers: &custom,
        params: REGION_PARAMS,
        data: r#"{"a":1}"#,
        api_base: "https://runtime.sagemaker.us-west-2.amazonaws.com/endpoints/x/invocations",
        fixture: &fixture,
        api_key: None,
        configured: None,
    })
    .unwrap();
    test_assert_signed(
        &signed,
        "https://runtime.sagemaker.us-west-2.amazonaws.com/endpoints/x/invocations",
        br#"{"a": 1}"#,
        &[
            ("Authorization", AUTH_SAGEMAKER),
            ("X-Amz-Date", "20260101T000000Z"),
            ("content-type", "application/x-custom"),
        ],
    );
}

fn null_content_type_suppresses_insertion() {
    let fixture = fixture_credentials("AKIDEXAMPLE", None);
    let null = supplied(&[("content-type", None)]);
    let signed = test_service(ServiceCase {
        service: ServiceName::S3vectors,
        headers: &null,
        params: REGION_PARAMS,
        data: r#"{"a":1}"#,
        api_base: "https://s3vectors.us-west-2.api.aws/vectors/x",
        fixture: &fixture,
        api_key: None,
        configured: None,
    })
    .unwrap();
    test_assert_signed(
        &signed,
        "https://s3vectors.us-west-2.api.aws/vectors/x",
        br#"{"a": 1}"#,
        &[
            ("Authorization", AUTH_S3VECTORS),
            ("X-Amz-Date", "20260101T000000Z"),
        ],
    );
}

fn body_service_checks() {
    bedrock_body_signature();
    sagemaker_keeps_content_type();
    null_content_type_suppresses_insertion();
}

fn body_json_post_checks() {
    let fixture = fixture_credentials("AKIDEXAMPLE", None);
    let headers = [
        ("content-type".to_owned(), "application/json".to_owned()),
        ("x-custom".to_owned(), "signed".to_owned()),
    ];
    let post = sign_aws_json_post(JsonPost {
        service_name: "bedrock",
        region: Some("us-west-2"),
        url: ENDPOINT,
        body: r#"{"a": 1}"#,
        headers: &headers,
        credentials: &fixture,
        now: now(),
    })
    .unwrap();
    assert_eq!(post.method, "POST");
    test_assert_signed(
        &post,
        ENDPOINT,
        br#"{"a": 1}"#,
        &[
            ("Authorization", AUTH_JSON_POST),
            ("Content-Length", "8"),
            ("X-Amz-Date", "20260101T000000Z"),
            ("content-type", "application/json"),
            ("x-custom", "signed"),
        ],
    );
    let missing = sign_aws_json_post(JsonPost {
        service_name: "bedrock",
        region: None,
        url: ENDPOINT,
        body: "{}",
        headers: &[],
        credentials: &fixture,
        now: now(),
    })
    .unwrap_err();
    assert!(matches!(missing, SigningError::MissingRegion));
}

fn body_bytes_match_signature() {
    assert_text_and_bytes_agree();
    body_service_checks();
    body_json_post_checks();
}

fn test_target_call(
    optional_params: &str,
    model: Option<&str>,
    bearer: Option<&str>,
) -> (
    BTreeMap<String, Box<RawValue>>,
    Result<RequestTarget, SigningError>,
    usize,
) {
    let fixture = fixture_credentials("AKIDEXAMPLE", Some("session-token"));
    let mut params = self::optional_params(optional_params);
    let calls = Cell::new(0_usize);
    let token = bearer.map(|text| SecretString::from(text.to_owned()));
    let calls_ref = &calls;
    let target = runtime().block_on(request_target(
        TargetRequest {
            optional_params: &mut params,
            model,
            region_inputs: RegionInputs::default(),
            bearer_token: token.as_ref(),
        },
        |_settings, _region| {
            calls_ref.set(calls_ref.get().saturating_add(1));
            ready(Ok::<Credentials, CredentialError>(fixture.clone()))
        },
    ));
    (params, target, calls.get())
}

fn target_bearer_takes_the_endpoint() {
    let full = r#"{"aws_region_name":"us-west-2","aws_access_key_id":"AKIDEXAMPLE","aws_secret_access_key":"wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY","aws_session_token":"session-token","aws_bedrock_runtime_endpoint":"https://custom.example","temperature":0.2}"#;
    let (remaining, result, resolved) = test_target_call(full, None, Some(""));
    let target = result.unwrap();
    assert_eq!(target.region, "us-west-2");
    assert_eq!(target.endpoint.as_deref(), Some("https://custom.example"));
    assert!(target.credentials.is_none());
    assert_eq!(resolved, 0);
    assert_eq!(
        remaining.keys().cloned().collect::<Vec<_>>(),
        ["temperature"]
    );
}

fn target_bearer_still_validates_auth() {
    let invalid_auth = r#"{"aws_region_name":"us-west-2","aws_access_key_id":{"nested":true}}"#;
    let (remaining, result, resolved) = test_target_call(invalid_auth, None, Some("tok"));
    result.unwrap_err();
    assert_eq!(resolved, 0);
    assert!(remaining.is_empty());
}

fn target_bearer_still_validates_region() {
    let invalid_region = r#"{"aws_region_name":"US-East-1"}"#;
    let (remaining, result, resolved) = test_target_call(invalid_region, None, Some(""));
    result.unwrap_err();
    assert_eq!(resolved, 0);
    assert!(remaining.contains_key("aws_region_name"));
}

fn target_bearer_still_validates_endpoint() {
    let bad_endpoint =
        r#"{"aws_region_name":"us-west-2","aws_bedrock_runtime_endpoint":{"nested":true}}"#;
    let (_, result, resolved) = test_target_call(bad_endpoint, None, Some("tok"));
    result.unwrap_err();
    assert_eq!(resolved, 0);
}

fn target_sigv4_resolves_credentials() {
    let full = r#"{"aws_region_name":"us-west-2","aws_access_key_id":"AKIDEXAMPLE","aws_secret_access_key":"wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY","aws_session_token":"session-token","aws_bedrock_runtime_endpoint":"https://custom.example","temperature":0.2}"#;
    let (remaining, result, resolved) = test_target_call(full, None, None);
    let target = result.unwrap();
    assert_eq!(
        target
            .credentials
            .unwrap()
            .session_token
            .unwrap()
            .expose_secret(),
        "session-token"
    );
    assert_eq!(resolved, 1);
    assert_eq!(
        remaining.keys().cloned().collect::<Vec<_>>(),
        ["temperature"]
    );
}

fn target_region_from_model_arn() {
    let arn = r#"{"aws_access_key_id":"AKIDEXAMPLE","aws_secret_access_key":"wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"}"#;
    let (remaining, result, resolved) = test_target_call(
        arn,
        Some("arn:aws:bedrock:eu-west-1:123456789012:inference-profile/example"),
        None,
    );
    let target = result.unwrap();
    assert_eq!(target.region, "eu-west-1");
    assert_eq!(target.endpoint, None);
    assert_eq!(
        target.credentials.unwrap().access_key_id.expose_secret(),
        "AKIDEXAMPLE"
    );
    assert_eq!(resolved, 1);
    assert!(remaining.is_empty());
}

fn auth_inputs_pop_even_bearer() {
    target_bearer_takes_the_endpoint();
    target_bearer_still_validates_auth();
    target_bearer_still_validates_region();
    target_bearer_still_validates_endpoint();
    target_sigv4_resolves_credentials();
    target_region_from_model_arn();
}

fn signing_off_loop() {
    let runtime = runtime();
    let _subscriber = set_default(registry());
    let span = tracing::info_span!("signing");
    let _guard = span.enter();
    let span_id = span.id();
    let executor = AwsSigningExecutor::default();
    let run_span = span_id;
    let run = runtime.block_on(async {
        let caller = current().id();
        executor
            .run(move || {
                (
                    tracing::Span::current().id() == run_span,
                    current().id() != caller,
                )
            })
            .await
            .unwrap()
    });
    assert_eq!(run, (true, true));
    let inline = off_loop_thread(&runtime, &executor, false);
    assert!(!inline);
    let offloaded = off_loop_thread(&runtime, &executor, true);
    assert!(offloaded);
    let failed = runtime
        .block_on(executor.run(|| Err::<(), &str>("signing failed")))
        .unwrap();
    assert_eq!(failed, Err("signing failed"));
}

fn off_loop_thread(runtime: &Runtime, executor: &AwsSigningExecutor, signs_with_aws: bool) -> bool {
    runtime.block_on(async {
        let caller = current().id();
        sign_request_off_loop_if_aws(executor, signs_with_aws, move || current().id() != caller)
            .await
            .unwrap()
    })
}

fn test_batch_get(endpoint: &str, fixture: &Credentials) {
    let ignored = request_data(r#"{"ignored":true}"#);
    let get = test_batch(BatchCase {
        service_name: "bedrock",
        data: BatchData::Json(&ignored),
        endpoint,
        params: REGION_PARAMS,
        method: "get",
        fixture,
    })
    .unwrap();
    assert_eq!(get.method, "GET");
    test_assert_signed(
        &get,
        endpoint,
        b"",
        &[
            ("Authorization", AUTH_BATCH_GET),
            ("X-Amz-Date", "20260101T000000Z"),
        ],
    );
}

fn test_batch_post(endpoint: &str, fixture: &Credentials) {
    let job = request_data(r#"{"jobName":"x"}"#);
    let post = test_batch(BatchCase {
        service_name: "bedrock",
        data: BatchData::Json(&job),
        endpoint,
        params: REGION_PARAMS,
        method: "POST",
        fixture,
    })
    .unwrap();
    assert_eq!(post.method, "POST");
    test_assert_signed(
        &post,
        endpoint,
        br#"{"jobName": "x"}"#,
        &[
            ("Authorization", AUTH_BATCH_POST),
            ("Content-Length", "16"),
            ("Content-Type", "application/json"),
            ("X-Amz-Date", "20260101T000000Z"),
        ],
    );
}

fn test_batch_put(fixture: &Credentials) {
    let put = test_batch(BatchCase {
        service_name: "s3",
        data: BatchData::Text("raw-body"),
        endpoint: "https://s3.us-west-2.amazonaws.com/bucket/key",
        params: TOKEN_PARAMS,
        method: "PUT",
        fixture,
    })
    .unwrap();
    assert_eq!(put.method, "PUT");
    test_assert_signed(
        &put,
        "https://s3.us-west-2.amazonaws.com/bucket/key",
        b"raw-body",
        &[
            ("Authorization", AUTH_BATCH_S3),
            ("Content-Length", "8"),
            ("Content-Type", "application/json"),
            ("X-Amz-Date", "20260101T000000Z"),
            ("X-Amz-Security-Token", "session-token"),
        ],
    );
}

fn test_batch_invalid(endpoint: &str, fixture: &Credentials) {
    let job = request_data(r#"{"jobName":"x"}"#);
    let invalid = r#"{"aws_access_key_id":{"bad":1},"aws_region_name":"us-west-2"}"#;
    let error = test_batch(BatchCase {
        service_name: "bedrock",
        data: BatchData::Json(&job),
        endpoint,
        params: invalid,
        method: "POST",
        fixture,
    })
    .unwrap_err();
    assert!(matches!(error, SigningError::Auth(_)));
}

fn batch_service_signing() {
    let fixture = fixture_credentials("AKIDEXAMPLE", None);
    let token_fixture = fixture_credentials("AKIDEXAMPLE", Some("session-token"));
    let endpoint = "https://bedrock.us-west-2.amazonaws.com/model-invocation-job";
    test_batch_get(endpoint, &fixture);
    test_batch_post(endpoint, &fixture);
    test_batch_put(&token_fixture);
    test_batch_invalid(endpoint, &fixture);
}

#[rstest]
#[case::fixed_clock_sigv4(fixed_clock_sigv4)]
#[case::session_token_signature(session_token_signature)]
#[case::signed_header_allowlist(signed_header_allowlist)]
#[case::computed_header_replacement(computed_header_replacement)]
#[case::bearer_skips_sts(bearer_skips_sts)]
#[case::explicit_non_aws_auth_override(explicit_non_aws_auth_override)]
#[case::empty_credentials_error(empty_credentials_error)]
#[case::body_bytes_match_signature(body_bytes_match_signature)]
#[case::auth_inputs_pop_even_bearer(auth_inputs_pop_even_bearer)]
#[case::signing_off_loop(signing_off_loop)]
#[case::batch_service_signing(batch_service_signing)]
fn signing_cases(#[case] scenario: fn()) {
    scenario();
}
