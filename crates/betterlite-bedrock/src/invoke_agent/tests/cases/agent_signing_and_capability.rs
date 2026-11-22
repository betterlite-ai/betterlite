use std::future::{Ready, ready};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::aws_settings::sources::Credentials;
use crate::bedrock_settings::{ErrorHeaders, HeaderError, HeaderValue};
use crate::credentials::error::CredentialError;
use crate::invoke_agent::error::InvokeAgentRequestError;
use crate::invoke_agent::payload::transform_request;
use crate::invoke_agent::policy::{SHOULD_FAKE_STREAM, SUPPORTED_OPENAI_PARAMS, get_error_class};
use crate::invoke_agent::prepare::{PrepareRequest, prepare_request};
use crate::invoke_agent::signing::{SignRequest, sign_request};
use crate::invoke_agent::tests::support::{
    block_on, fixture_credentials, generated_uuid, header, headers, messages, now, params,
    request_data, secret,
};
use crate::invoke_agent::url::InvokeEnvironment;
use crate::model_routes::ParamMap;
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::RegionInputs;

const MODEL: &str = "agent/L1RT58GYRW/MFPSBCXYTW";
const API_BASE: &str = "https://bedrock-agent-runtime.us-east-1.amazonaws.com/agents/L1RT58GYRW/agentAliases/MFPSBCXYTW/sessions/123e4567-e89b-12d3-a456-426614174000/text";
const AWS_SECRET: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
const JWT: &str = "jwt-fixture-token";

type Resolution = Ready<Result<Credentials, CredentialError>>;
type Resolver = fn(&AwsSettings, &str) -> Resolution;

#[derive(Clone, Copy)]
struct SignCase<'text> {
    headers: &'text [(String, Option<String>)],
    optional_params: &'text ParamMap,
    request_data: &'text IndexMap<String, Json>,
    api_key: Option<&'text SecretString>,
    configured_bearer: Option<&'text SecretString>,
}

fn sign<R>(case: SignCase<'_>, resolve: R) -> Result<AwsSignedRequest, InvokeAgentRequestError>
where
    R: FnOnce(&AwsSettings, &str) -> Resolution,
{
    block_on(sign_request(
        SignRequest {
            headers: case.headers,
            optional_params: case.optional_params,
            request_data: case.request_data,
            api_base: API_BASE,
            api_key: case.api_key,
            configured_bearer: case.configured_bearer,
            model: Some(MODEL),
            stream: Some(true),
            fake_stream: None,
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

pub(super) fn agent_signing_and_capability() {
    assert_capability_policy();
    assert_optional_params_and_headers_are_untouched();
    assert_error_factory();
    assert_bearer_signing();
    assert_configured_bearer();
    assert_sigv4_signing();
    assert_empty_key_is_not_a_bearer();
    assert_session_token_signing();
    assert_missing_credentials();
    assert_prepared_pipeline();
}

fn assert_capability_policy() {
    let supported = SUPPORTED_OPENAI_PARAMS;
    assert!(supported.is_empty(), "InvokeAgent maps no OpenAI parameter");
    let fake_stream = SHOULD_FAKE_STREAM;
    assert!(
        fake_stream,
        "InvokeAgent always answers through the fake-stream wrapper"
    );
}

fn assert_optional_params_and_headers_are_untouched() {
    let supplied = params(
        r#"{"inputText":"override","temperature":0.5,"stream":true,"sessionID":"s","aws_access_key_id":"AKID"}"#,
    );
    let before = supplied.clone();
    let payload =
        transform_request(&messages(r#"[{"role":"user","content":"hi"}]"#), &supplied).unwrap();
    assert_eq!(
        payload,
        request_data(
            r#"{"inputText":"override","enableTrace":true,"temperature":0.5,"stream":true,"sessionID":"s","aws_access_key_id":"AKID"}"#
        )
    );
    assert_eq!(
        supplied, before,
        "mapping leaves the optional map unchanged"
    );
    let caller = headers(&[
        ("x-request-id", Some("abc")),
        ("Accept", Some("text/plain")),
    ]);
    let before_headers = caller.clone();
    let data = request_data(r#"{"inputText":"hello"}"#);
    let optional = params("{}");
    let token = secret(JWT);
    sign(
        SignCase {
            headers: &caller,
            optional_params: &optional,
            request_data: &data,
            api_key: Some(&token),
            configured_bearer: None,
        },
        failing(),
    )
    .unwrap();
    assert_eq!(
        caller, before_headers,
        "environment validation leaves the caller headers in place"
    );
}

fn assert_error_factory() {
    let supplied = ErrorHeaders::Http(vec![
        ("x-request-id".to_owned(), "abc".to_owned()),
        ("retry-after".to_owned(), "3".to_owned()),
    ]);
    let error = get_error_class("upstream refused".to_owned(), 429, Some(supplied)).unwrap();
    assert_eq!(error.error.status_code, 429);
    assert_eq!(error.error.response.status_code, 429);
    assert!(!error.error.status_code_is_synthesized);
    assert_eq!(
        error.response_headers,
        vec![
            ("x-request-id".to_owned(), "abc".to_owned()),
            ("retry-after".to_owned(), "3".to_owned())
        ]
    );
    let non_ascii = ErrorHeaders::Mapping(vec![(
        "x-\u{e9}".to_owned(),
        HeaderValue::Text("x".to_owned()),
    )]);
    assert!(matches!(
        get_error_class("bad".to_owned(), 400, Some(non_ascii)),
        Err(HeaderError::NotAscii { name }) if name == "x-\u{e9}"
    ));
}

fn assert_bearer_signing() {
    let data = request_data(r#"{"inputText":"hello","enableTrace":true}"#);
    let supplied = headers(&[("x-request-id", Some("abc"))]);
    let optional = params("{}");
    let explicit = secret(JWT);
    let case = SignCase {
        headers: &supplied,
        optional_params: &optional,
        request_data: &data,
        api_key: Some(&explicit),
        configured_bearer: None,
    };
    let signed = sign(case, failing()).unwrap();
    assert_eq!(signed.method, "POST");
    assert_eq!(signed.url, API_BASE);
    assert_eq!(
        signed.body,
        br#"{"inputText": "hello", "enableTrace": true}"#
    );
    assert_eq!(
        header(&signed.headers, "Content-Type"),
        Some("application/json")
    );
    assert_eq!(
        header(&signed.headers, "Authorization"),
        Some(format!("Bearer {JWT}").as_str())
    );
    assert_eq!(header(&signed.headers, "x-request-id"), Some("abc"));
    assert_eq!(header(&signed.headers, "X-Amz-Date"), None);
}

fn assert_configured_bearer() {
    let data = request_data(r#"{"inputText":"hello"}"#);
    let supplied = headers(&[]);
    let optional = params("{}");
    let configured = secret("env-bearer-token");
    let case = SignCase {
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
    assert_eq!(header(&signed.headers, "X-Amz-Date"), None);
}

fn assert_sigv4_signing() {
    let data = request_data(r#"{"inputText":"hello"}"#);
    let supplied = headers(&[("x-request-id", Some("abc"))]);
    let optional = params(&format!(
        r#"{{"aws_access_key_id":"AKIDEXAMPLE","aws_secret_access_key":"{AWS_SECRET}","aws_region_name":"us-east-1"}}"#
    ));
    let case = SignCase {
        headers: &supplied,
        optional_params: &optional,
        request_data: &data,
        api_key: None,
        configured_bearer: None,
    };
    let signed = sign(case, resolver(fixture_credentials("AKIDEXAMPLE"))).unwrap();
    assert_eq!(
        header(&signed.headers, "X-Amz-Date"),
        Some("20260101T000000Z")
    );
    assert_eq!(
        header(&signed.headers, "Content-Type"),
        Some("application/json")
    );
    assert_eq!(header(&signed.headers, "x-request-id"), Some("abc"));
    let authorization = header(&signed.headers, "Authorization").unwrap();
    assert!(
        authorization.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-east-1/bedrock/aws4_request"
        ),
        "{authorization}"
    );
    assert!(
        authorization.contains("SignedHeaders=content-type;host;x-amz-date"),
        "{authorization}"
    );
}

fn assert_empty_key_is_not_a_bearer() {
    let data = request_data(r#"{"inputText":"hello"}"#);
    let supplied = headers(&[("x-request-id", Some("abc"))]);
    let optional = params(&format!(
        r#"{{"aws_access_key_id":"AKIDEXAMPLE","aws_secret_access_key":"{AWS_SECRET}","aws_region_name":"us-east-1"}}"#
    ));
    let empty = secret("");
    let signed_with_empty_key = sign(
        SignCase {
            headers: &supplied,
            optional_params: &optional,
            request_data: &data,
            api_key: Some(&empty),
            configured_bearer: Some(&secret("env-bearer-token")),
        },
        resolver(fixture_credentials("AKIDEXAMPLE")),
    )
    .unwrap();
    assert!(
        header(&signed_with_empty_key.headers, "Authorization")
            .is_some_and(|value| value.starts_with("AWS4-HMAC-SHA256")),
        "an empty explicit key does not become a bearer"
    );
}

fn assert_session_token_signing() {
    let data = request_data(r#"{"inputText":"hello"}"#);
    let supplied = headers(&[]);
    let optional = params(&format!(
        r#"{{"aws_access_key_id":"AKIDEXAMPLE","aws_secret_access_key":"{AWS_SECRET}","aws_region_name":"us-east-1"}}"#
    ));
    let case = SignCase {
        headers: &supplied,
        optional_params: &optional,
        request_data: &data,
        api_key: None,
        configured_bearer: None,
    };
    let session = Credentials {
        access_key_id: secret("AKIDEXAMPLE"),
        secret_access_key: secret(AWS_SECRET),
        session_token: Some(secret("session-token")),
    };
    let signed = sign(case, resolver(session)).unwrap();
    assert_eq!(
        header(&signed.headers, "X-Amz-Security-Token"),
        Some("session-token")
    );
    assert!(
        header(&signed.headers, "Authorization")
            .is_some_and(|value| value.contains("x-amz-security-token")),
        "the session token joins the signature"
    );
}

fn assert_missing_credentials() {
    let data = request_data(r#"{"inputText":"hello"}"#);
    let supplied = headers(&[]);
    let optional = params(r#"{"aws_access_key_id":"AKIDEXAMPLE","aws_region_name":"us-east-1"}"#);
    let case = SignCase {
        headers: &supplied,
        optional_params: &optional,
        request_data: &data,
        api_key: None,
        configured_bearer: None,
    };
    assert!(matches!(
        sign(case, failing()),
        Err(InvokeAgentRequestError::Signing { .. })
    ));
}

fn assert_prepared_pipeline() {
    let context = messages(r#"[{"role":"user","content":"hi"}]"#);
    let supplied = headers(&[]);
    let optional = params(r#"{"sessionID":"session-1"}"#);
    let explicit = secret(JWT);
    let signed = block_on(prepare_request(
        PrepareRequest {
            model: MODEL,
            messages: &context,
            optional_params: &optional,
            headers: &supplied,
            api_base: None,
            environment: InvokeEnvironment::default(),
            api_key: Some(&explicit),
            configured_bearer: None,
            stream: Some(true),
            fake_stream: Some(true),
            now: now(),
            generated_session_id: generated_uuid(),
        },
        failing(),
    ))
    .unwrap();
    assert_eq!(
        signed.url,
        "https://bedrock-agent-runtime.us-west-2.amazonaws.com/agents/L1RT58GYRW/agentAliases/MFPSBCXYTW/sessions/session-1/text"
    );
    assert_eq!(
        signed.body,
        br#"{"inputText": "hi", "enableTrace": true, "sessionID": "session-1"}"#
    );
    assert_eq!(
        header(&signed.headers, "Authorization"),
        Some(format!("Bearer {JWT}").as_str())
    );
    assert_eq!(
        header(&signed.headers, "Content-Type"),
        Some("application/json")
    );
}
