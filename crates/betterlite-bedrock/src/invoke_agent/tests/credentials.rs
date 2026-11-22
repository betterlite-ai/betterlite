use std::collections::BTreeMap;
use std::future::ready;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::SecretString;

use crate::aws_settings::credential_cache::{Parameter, cache_key};
use crate::aws_settings::sources::Credentials;
use crate::bedrock_settings::ErrorHeaders;
use crate::credentials::error::CredentialError;
use crate::invoke_agent::error::InvokeAgentRequestError;
use crate::invoke_agent::policy::get_error_class;
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

#[test]
fn invoke_agent_credential_identity_isolation() {
    assert_signed_headers_follow_the_identity();
    assert_bearers_stay_separate();
    assert_cache_keys_are_distinct();
}

#[test]
fn invoke_agent_secret_safe_failure() {
    assert_signed_request_debug_is_secret_safe();
    assert_failures_are_secret_safe();
    assert_bedrock_error_debug_is_secret_safe();
}

fn signed_with<R>(credentials: R, optional_params: &ParamMap) -> AwsSignedRequest
where
    R: FnOnce() -> Credentials,
{
    let data = request_data(r#"{"inputText":"hello"}"#);
    let supplied = headers(&[]);
    block_on(sign_request(
        SignRequest {
            headers: &supplied,
            optional_params,
            request_data: &data,
            api_base: API_BASE,
            api_key: None,
            configured_bearer: None,
            model: Some(MODEL),
            stream: None,
            fake_stream: None,
            region_inputs: RegionInputs::default(),
            now: now(),
        },
        move |_settings: &AwsSettings, _region: &str| ready(Ok(credentials())),
    ))
    .unwrap()
}

fn signed_with_bearer(token: &SecretString) -> AwsSignedRequest {
    let data = request_data(r#"{"inputText":"hello"}"#);
    let supplied = headers(&[]);
    let optional = params("{}");
    block_on(sign_request(
        SignRequest {
            headers: &supplied,
            optional_params: &optional,
            request_data: &data,
            api_base: API_BASE,
            api_key: Some(token),
            configured_bearer: None,
            model: Some(MODEL),
            stream: None,
            fake_stream: None,
            region_inputs: RegionInputs::default(),
            now: now(),
        },
        |_settings: &AwsSettings, _region: &str| ready(Err(CredentialError::MissingCredentials)),
    ))
    .unwrap()
}

fn assert_signed_headers_follow_the_identity() {
    let optional = params(r#"{"aws_region_name":"us-east-1"}"#);
    let first = signed_with(|| fixture_credentials("AKIDFIRST"), &optional);
    let again = signed_with(|| fixture_credentials("AKIDFIRST"), &optional);
    let second = signed_with(|| fixture_credentials("AKIDSECOND"), &optional);
    assert_eq!(
        first.headers, again.headers,
        "one identity signs the same request the same way"
    );
    assert_ne!(
        first.headers, second.headers,
        "two identities cannot share signed headers"
    );
    assert_eq!(first.body, second.body, "the request body is the same");
    assert!(
        header(&first.headers, "Authorization")
            .is_some_and(|value| value.contains("Credential=AKIDFIRST/"))
    );
    assert!(
        header(&second.headers, "Authorization")
            .is_some_and(|value| value.contains("Credential=AKIDSECOND/"))
    );
}

fn assert_bearers_stay_separate() {
    let first = signed_with_bearer(&secret("bearer-first"));
    let again = signed_with_bearer(&secret("bearer-first"));
    let second = signed_with_bearer(&secret("bearer-second"));
    assert_eq!(first.headers, again.headers);
    assert_ne!(first.headers, second.headers);
    assert_eq!(
        header(&first.headers, "Authorization"),
        Some("Bearer bearer-first")
    );
    assert_eq!(
        header(&second.headers, "Authorization"),
        Some("Bearer bearer-second")
    );
}

fn credential_parameters(
    access: &str,
    secret_text: &str,
    token: &str,
) -> BTreeMap<String, Parameter> {
    BTreeMap::from([
        (
            "aws_access_key_id".to_owned(),
            Parameter::Text(SecretString::from(access.to_owned())),
        ),
        (
            "aws_secret_access_key".to_owned(),
            Parameter::Text(SecretString::from(secret_text.to_owned())),
        ),
        (
            "aws_session_token".to_owned(),
            Parameter::Text(SecretString::from(token.to_owned())),
        ),
        ("aws_region_name".to_owned(), Parameter::Null),
    ])
}

fn assert_cache_keys_are_distinct() {
    let first = cache_key(&credential_parameters("AKIDFIRST", AWS_SECRET, "token-1")).unwrap();
    let first_again =
        cache_key(&credential_parameters("AKIDFIRST", AWS_SECRET, "token-1")).unwrap();
    let other_access =
        cache_key(&credential_parameters("AKIDSECOND", AWS_SECRET, "token-1")).unwrap();
    let other_secret = cache_key(&credential_parameters(
        "AKIDFIRST",
        "another-secret",
        "token-1",
    ))
    .unwrap();
    let other_token =
        cache_key(&credential_parameters("AKIDFIRST", AWS_SECRET, "token-2")).unwrap();
    assert_eq!(first, first_again, "one identity keeps one cache entry");
    assert_ne!(first, other_access);
    assert_ne!(first, other_secret);
    assert_ne!(first, other_token);
}

fn assert_signed_request_debug_is_secret_safe() {
    let optional = params(&format!(
        r#"{{"aws_region_name":"us-east-1","aws_secret_access_key":"{AWS_SECRET}"}}"#
    ));
    let signed = signed_with(|| fixture_credentials("AKIDFIRST"), &optional);
    let debug = format!("{signed:?}");
    assert!(!debug.contains(AWS_SECRET));
    assert!(!debug.contains("AKIDFIRST"));
    assert!(debug.contains("body_length"));
}

fn optional_with_region(region: &str) -> ParamMap {
    params(&format!(
        r#"{{"aws_region_name":"{region}","aws_access_key_id":"AKIDFIRST","aws_secret_access_key":"{AWS_SECRET}"}}"#
    ))
}

fn sign_failure() -> InvokeAgentRequestError {
    let data = request_data(r#"{"inputText":"hello"}"#);
    let supplied = headers(&[]);
    let optional = optional_with_region("US_EAST");
    block_on(sign_request(
        SignRequest {
            headers: &supplied,
            optional_params: &optional,
            request_data: &data,
            api_base: API_BASE,
            api_key: None,
            configured_bearer: None,
            model: Some(MODEL),
            stream: None,
            fake_stream: None,
            region_inputs: RegionInputs::default(),
            now: now(),
        },
        |_settings: &AwsSettings, _region: &str| ready(Ok(fixture_credentials("AKIDFIRST"))),
    ))
    .unwrap_err()
}

fn prepare_failure() -> InvokeAgentRequestError {
    let context = messages(r#"[{"role":"user","content":"hi"}]"#);
    let supplied = headers(&[]);
    let optional = optional_with_region("us-east-1");
    block_on(prepare_request(
        PrepareRequest {
            model: "not-agent/one",
            messages: &context,
            optional_params: &optional,
            headers: &supplied,
            api_base: None,
            environment: InvokeEnvironment::default(),
            api_key: None,
            configured_bearer: None,
            stream: None,
            fake_stream: None,
            now: now(),
            generated_session_id: generated_uuid(),
        },
        |_settings: &AwsSettings, _region: &str| ready(Ok(fixture_credentials("AKIDFIRST"))),
    ))
    .unwrap_err()
}

fn assert_failures_are_secret_safe() {
    let sign_error = sign_failure();
    assert!(matches!(
        &sign_error,
        InvokeAgentRequestError::Signing { .. }
    ));
    assert_secret_free(&sign_error);
    let prepare_error = prepare_failure();
    assert!(matches!(
        &prepare_error,
        InvokeAgentRequestError::InvalidModelFormat
    ));
    assert_secret_free(&prepare_error);
}

fn assert_secret_free(error: &InvokeAgentRequestError) {
    let display = format!("{error}");
    let debug = format!("{error:?}");
    assert!(!display.contains(AWS_SECRET), "{display}");
    assert!(!debug.contains(AWS_SECRET), "{debug}");
    assert!(!display.contains("AKIDFIRST"), "{display}");
    assert!(!debug.contains("AKIDFIRST"), "{debug}");
}

fn assert_bedrock_error_debug_is_secret_safe() {
    let headers = ErrorHeaders::Http(vec![("Authorization".to_owned(), format!("Bearer {JWT}"))]);
    let error = get_error_class("upstream refused".to_owned(), 429, Some(headers)).unwrap();
    let debug = format!("{error:?}");
    assert!(!debug.contains(JWT), "{debug}");
}
