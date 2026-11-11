use std::future::{Ready, ready};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::agentcore_request::error::AgentCoreRequestError;
use crate::agentcore_request::identifiers::{SESSION_HEADER, USER_HEADER};
use crate::agentcore_request::prepare::{PrepareRequest, prepare_request};
use crate::agentcore_request::signing::{SignRequest, sign_request};
use crate::agentcore_request::tests::support::{
    SESSION_UUID, block_on, fixture_credentials, generated_uuid, header, headers, messages, now,
    params, request_data, secret,
};
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::model_routes::ParamMap;
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::RegionInputs;

const MODEL: &str =
    "agentcore/arn:aws:bedrock-agentcore:us-west-2:123456789012:runtime/agent_runtime";
const API_BASE: &str = "https://bedrock-agentcore.us-west-2.amazonaws.com/runtimes/arn%3Aaws%3Abedrock-agentcore%3Aus-west-2%3A123456789012%3Aruntime%2Fagent_runtime/invocations";
const JWT: &str = "jwt-fixture-token";
const AWS_SECRET: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
const IAM_AUTHORIZATION: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-east-1/bedrock-agentcore/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amzn-bedrock-agentcore-runtime-session-id;x-amzn-bedrock-agentcore-runtime-user-id, Signature=960beacfd020969cb7f5eb37648dd7b84bc0a1a58af038a1dde5c37ff245cee1";

type Resolution = Ready<Result<Credentials, CredentialError>>;
type Resolver = fn(&AwsSettings, &str) -> Resolution;

#[derive(Clone, Copy)]
struct Case<'text> {
    headers: &'text [(String, Option<String>)],
    optional_params: &'text ParamMap,
    request_data: &'text IndexMap<String, Json>,
    api_key: Option<&'text SecretString>,
    configured_bearer: Option<&'text SecretString>,
}

fn sign<R>(case: Case<'_>, resolve: R) -> Result<AwsSignedRequest, AgentCoreRequestError>
where
    R: FnOnce(&AwsSettings, &str) -> Resolution,
{
    block_on(sign_request(
        SignRequest {
            headers: case.headers,
            optional_params: case.optional_params,
            request_data: case.request_data,
            api_base: API_BASE,
            model: Some(MODEL),
            api_key: case.api_key,
            configured_bearer: case.configured_bearer,
            region_inputs: RegionInputs::default(),
            now: now(),
        },
        resolve,
    ))
}

fn resolver(credentials: Credentials) -> impl FnOnce(&AwsSettings, &str) -> Resolution {
    move |_settings: &AwsSettings, _region: &str| ready(Ok(credentials))
}

fn failing() -> Resolver {
    |_settings: &AwsSettings, _region: &str| ready(Err(CredentialError::MissingCredentials))
}

fn bearer_optional() -> ParamMap {
    params(&format!(
        r#"{{"api_key":"{JWT}-optional","aws_access_key_id":"AKID","aws_secret_access_key":"{AWS_SECRET}"}}"#
    ))
}

fn iam_optional() -> ParamMap {
    params(&format!(
        r#"{{"aws_access_key_id":"AKIDEXAMPLE","aws_secret_access_key":"{AWS_SECRET}","aws_region_name":"us-east-1"}}"#
    ))
}

fn bearer_case<'text>(
    headers: &'text [(String, Option<String>)],
    optional_params: &'text ParamMap,
    request_data: &'text IndexMap<String, Json>,
    api_key: Option<&'text SecretString>,
) -> Case<'text> {
    Case {
        headers,
        optional_params,
        request_data,
        api_key,
        configured_bearer: None,
    }
}

pub(super) fn agentcore_bearer_signing() {
    assert_explicit_bearer_wins();
    assert_optional_bearer_fallback();
    assert_bearer_requires_a_token();
    assert_prepared_pipeline();
}

fn assert_explicit_bearer_wins() {
    let data = request_data(r#"{"prompt":"hello","nested":{"a":[1,2]},"flag":true}"#);
    let body = br#"{"prompt": "hello", "nested": {"a": [1, 2]}, "flag": true}"#;
    let supplied = headers(&[
        ("Accept", Some("text/plain")),
        ("x-request-id", Some("abc")),
    ]);
    let explicit = secret(&format!("{JWT}-explicit"));
    let optional = bearer_optional();
    let signed = sign(
        bearer_case(&supplied, &optional, &data, Some(&explicit)),
        failing(),
    )
    .unwrap();
    assert_eq!(signed.method, "POST");
    assert_eq!(signed.url, API_BASE);
    assert_eq!(signed.body, body);
    assert_eq!(
        header(&signed.headers, "Accept"),
        Some("application/json, text/event-stream")
    );
    assert_eq!(
        header(&signed.headers, "Content-Type"),
        Some("application/json")
    );
    assert_eq!(
        header(&signed.headers, "Authorization"),
        Some(format!("Bearer {JWT}-explicit").as_str())
    );
    assert_eq!(header(&signed.headers, "x-request-id"), Some("abc"));
}

fn assert_optional_bearer_fallback() {
    let data = request_data(r#"{"prompt":"hello","nested":{"a":[1,2]},"flag":true}"#);
    let supplied = headers(&[]);
    let optional = bearer_optional();
    let empty = secret("");
    let expected = format!("Bearer {JWT}-optional");
    let signed = sign(
        bearer_case(&supplied, &optional, &data, Some(&empty)),
        failing(),
    )
    .unwrap();
    assert_eq!(
        header(&signed.headers, "Authorization"),
        Some(expected.as_str()),
        "an empty explicit key falls back to the optional one"
    );
    let fallback = sign(bearer_case(&supplied, &optional, &data, None), failing()).unwrap();
    assert_eq!(
        header(&fallback.headers, "Authorization"),
        Some(expected.as_str())
    );
}

fn assert_bearer_requires_a_token() {
    let data = request_data(r#"{"prompt":"hello"}"#);
    let supplied = headers(&[]);
    assert!(matches!(
        sign(
            bearer_case(&supplied, &params(r#"{"api_key":7}"#), &data, None),
            resolver(fixture_credentials("AKIDEXAMPLE"))
        ),
        Err(AgentCoreRequestError::WrongType { field: "api_key" })
    ));
    let signed = sign(
        bearer_case(&supplied, &params(r#"{"api_key":""}"#), &data, None),
        resolver(fixture_credentials("AKIDEXAMPLE")),
    )
    .unwrap();
    assert!(
        header(&signed.headers, "Authorization")
            .is_some_and(|value| value.starts_with("AWS4-HMAC-SHA256")),
        "an empty optional key delegates to SigV4"
    );
}

fn assert_prepared_pipeline() {
    let context = messages(r#"[{"role":"user","content":"hi"}]"#);
    let supplied = headers(&[]);
    let optional = bearer_optional();
    let explicit = secret(&format!("{JWT}-explicit"));
    let signed = block_on(prepare_request(
        PrepareRequest {
            model: MODEL,
            messages: &context,
            optional_params: &optional,
            litellm_params: &params("{}"),
            headers: &supplied,
            api_key: Some(&explicit),
            configured_bearer: None,
            region_inputs: RegionInputs::default(),
            now: now(),
            generated_session_id: generated_uuid(),
        },
        failing(),
    ))
    .unwrap();
    assert_eq!(signed.url, API_BASE);
    assert_eq!(signed.body, br#"{"prompt": "hi"}"#);
    assert_eq!(
        header(&signed.headers, "Authorization"),
        Some(format!("Bearer {JWT}-explicit").as_str())
    );
    assert_eq!(
        header(&signed.headers, SESSION_HEADER),
        Some(format!("litellm-session-{SESSION_UUID}").as_str())
    );
}

pub(super) fn agentcore_iam_signing() {
    assert_static_signature();
    assert_session_token_signature();
    assert_configured_bearer();
    assert_missing_credentials_fail();
}

fn iam_headers() -> Vec<(String, Option<String>)> {
    headers(&[
        (SESSION_HEADER, Some("session-1")),
        (USER_HEADER, Some("user-1")),
        ("Accept", Some("text/plain")),
    ])
}

fn assert_static_signature() {
    let data = request_data(r#"{"prompt":"hello"}"#);
    let supplied = iam_headers();
    let optional = iam_optional();
    let signed = sign(
        bearer_case(&supplied, &optional, &data, None),
        resolver(fixture_credentials("AKIDEXAMPLE")),
    )
    .unwrap();
    assert_eq!(signed.method, "POST");
    assert_eq!(signed.url, API_BASE);
    assert_eq!(signed.body, br#"{"prompt": "hello"}"#);
    assert_eq!(
        header(&signed.headers, "X-Amz-Date"),
        Some("20260101T000000Z")
    );
    assert_eq!(
        header(&signed.headers, "Accept"),
        Some("application/json, text/event-stream")
    );
    assert_eq!(header(&signed.headers, SESSION_HEADER), Some("session-1"));
    assert_eq!(header(&signed.headers, USER_HEADER), Some("user-1"));
    assert_eq!(header(&signed.headers, "X-Amz-Security-Token"), None);
    assert_eq!(
        header(&signed.headers, "Authorization"),
        Some(IAM_AUTHORIZATION),
        "the URL region is us-west-2 while the signature uses the aws_region_name us-east-1"
    );
}

fn assert_session_token_signature() {
    let data = request_data(r#"{"prompt":"hello"}"#);
    let supplied = iam_headers();
    let optional = iam_optional();
    let empty = secret("");
    let signed = sign(
        bearer_case(&supplied, &optional, &data, Some(&empty)),
        resolver(fixture_credentials("AKIDEXAMPLE")),
    )
    .unwrap();
    assert!(
        header(&signed.headers, "Authorization")
            .is_some_and(|value| value.starts_with("AWS4-HMAC-SHA256")),
        "an empty explicit key does not become a bearer"
    );
    let session = Credentials {
        access_key_id: secret("AKIDEXAMPLE"),
        secret_access_key: secret(AWS_SECRET),
        session_token: Some(secret("session-token")),
    };
    let with_token = sign(
        bearer_case(&supplied, &optional, &data, None),
        resolver(session),
    )
    .unwrap();
    assert_eq!(
        header(&with_token.headers, "X-Amz-Security-Token"),
        Some("session-token")
    );
    assert!(
        header(&with_token.headers, "Authorization")
            .is_some_and(|value| value.contains("x-amz-security-token")),
        "the session token joins the signature"
    );
}

fn assert_configured_bearer() {
    let data = request_data(r#"{"prompt":"hello"}"#);
    let supplied = iam_headers();
    let optional = iam_optional();
    let configured = secret("env-bearer-token");
    let case = Case {
        headers: &supplied,
        optional_params: &optional,
        request_data: &data,
        api_key: None,
        configured_bearer: Some(&configured),
    };
    let signed = sign(case, failing()).unwrap();
    assert_eq!(
        header(&signed.headers, "Authorization"),
        Some("Bearer env-bearer-token")
    );
    assert_eq!(signed.body, br#"{"prompt": "hello"}"#);
    assert_eq!(
        header(&signed.headers, "Content-Type"),
        Some("application/json")
    );
    assert_eq!(header(&signed.headers, "X-Amz-Date"), None);
}

fn assert_missing_credentials_fail() {
    let data = request_data(r#"{"prompt":"hello"}"#);
    let supplied = headers(&[]);
    let optional = params(r#"{"aws_access_key_id":"AKIDEXAMPLE","aws_region_name":"us-east-1"}"#);
    assert!(matches!(
        sign(bearer_case(&supplied, &optional, &data, None), failing()),
        Err(AgentCoreRequestError::Signing { .. })
    ));
}
