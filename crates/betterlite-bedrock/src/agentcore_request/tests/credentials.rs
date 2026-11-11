use std::future::{Ready, ready};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::agentcore_request::error::AgentCoreRequestError;
use crate::agentcore_request::payload::transform_request;
use crate::agentcore_request::signing::{SignRequest, sign_request};
use crate::agentcore_request::tests::support::{
    block_on, generated_uuid, header, headers, messages, now, params, request_data, secret,
};
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::model_routes::ParamMap;
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::RegionInputs;

const MODEL: &str =
    "agentcore/arn:aws:bedrock-agentcore:us-west-2:123456789012:runtime/agent_runtime";
const API_BASE: &str = "https://bedrock-agentcore.us-west-2.amazonaws.com/runtimes/arn%3Aaws%3Abedrock-agentcore%3Aus-west-2%3A123456789012%3Aruntime%2Fagent_runtime/invocations";
const AWS_SECRET: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";

type Resolution = Ready<Result<Credentials, CredentialError>>;

fn sign<R>(
    optional_params: &ParamMap,
    request_data: &IndexMap<String, Json>,
    api_key: Option<&SecretString>,
    resolve: R,
) -> Result<AwsSignedRequest, AgentCoreRequestError>
where
    R: FnOnce(&AwsSettings, &str) -> Resolution,
{
    let supplied = headers(&[]);
    block_on(sign_request(
        SignRequest {
            headers: &supplied,
            optional_params,
            request_data,
            api_base: API_BASE,
            model: Some(MODEL),
            api_key,
            configured_bearer: None,
            region_inputs: RegionInputs::default(),
            now: now(),
        },
        resolve,
    ))
}

fn aws_credentials(access: &str) -> Credentials {
    Credentials {
        access_key_id: secret(access),
        secret_access_key: secret(AWS_SECRET),
        session_token: None,
    }
}

fn authorization(signed: &AwsSignedRequest) -> String {
    header(&signed.headers, "Authorization")
        .expect("a signed request carries Authorization")
        .to_owned()
}

#[test]
fn bedrock_services_agentcore_request_credential_identity_isolation() {
    assert_bearer_headers_are_isolated();
    assert_sigv4_credentials_are_isolated();
}

fn assert_bearer_headers_are_isolated() {
    let data = request_data(r#"{"prompt":"hi"}"#);
    let first = secret("jwt-first-identity");
    let second = secret("jwt-second-identity");
    let optional = params("{}");
    let first_signed = sign(&optional, &data, Some(&first), failing()).unwrap();
    let second_signed = sign(&optional, &data, Some(&second), failing()).unwrap();
    assert_eq!(
        authorization(&first_signed),
        "Bearer jwt-first-identity",
        "one identity never reuses the other's bearer header"
    );
    assert_eq!(authorization(&second_signed), "Bearer jwt-second-identity");
    let repeated = sign(&optional, &data, Some(&first), failing()).unwrap();
    assert_eq!(
        authorization(&repeated),
        authorization(&first_signed),
        "the mapper keeps no state between requests"
    );
}

fn assert_sigv4_credentials_are_isolated() {
    let data = request_data(r#"{"prompt":"hi"}"#);
    let optional = params(&format!(
        r#"{{"aws_access_key_id":"AKIDFIRST","aws_secret_access_key":"{AWS_SECRET}","aws_region_name":"us-east-1"}}"#
    ));
    let first = sign(
        &optional,
        &data,
        None,
        resolver(aws_credentials("AKIDFIRST")),
    )
    .unwrap();
    let second = sign(
        &optional,
        &data,
        None,
        resolver(aws_credentials("AKIDSECOND")),
    )
    .unwrap();
    let first_authorization = authorization(&first);
    let second_authorization = authorization(&second);
    assert!(
        first_authorization.contains("Credential=AKIDFIRST/"),
        "{first_authorization}"
    );
    assert!(!first_authorization.contains("AKIDSECOND"));
    assert!(
        second_authorization.contains("Credential=AKIDSECOND/"),
        "{second_authorization}"
    );
    assert!(!second_authorization.contains("AKIDFIRST"));
    assert_ne!(first_authorization, second_authorization);
}

#[test]
fn bedrock_services_agentcore_request_secret_safe_failure() {
    assert_signed_debug_names_headers_only();
    assert_failure_carries_no_secret();
    assert_payload_carries_no_credentials();
}

fn assert_signed_debug_names_headers_only() {
    let data = request_data(r#"{"prompt":"hi"}"#);
    let token = secret("sk-fixture-secret-token");
    let signed = sign(&params("{}"), &data, Some(&token), failing()).unwrap();
    let debug = format!("{signed:?}");
    assert!(!debug.contains("sk-fixture-secret-token"), "{debug}");
    assert!(debug.contains("header_names"), "{debug}");
}

fn assert_failure_carries_no_secret() {
    let data = request_data(r#"{"prompt":"hi"}"#);
    let optional = params(&format!(
        r#"{{"aws_access_key_id":"AKIDEXAMPLE","aws_secret_access_key":"{AWS_SECRET}","aws_region_name":"us-east-1"}}"#
    ));
    let error = sign(&optional, &data, None, failing()).unwrap_err();
    let rendered = format!("{error} {error:?}");
    assert!(!rendered.contains(AWS_SECRET), "{rendered}");
}

fn assert_payload_carries_no_credentials() {
    let mut request_headers = headers(&[]);
    let payload = transform_request(
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        &params(r#"{"runtimeUserId":"user-secret-free"}"#),
        &params("{}"),
        &mut request_headers,
        generated_uuid(),
    )
    .unwrap();
    assert_eq!(payload.len(), 1);
    assert!(!format!("{payload:?}").contains("sk-fixture"));
}

fn resolver(credentials: Credentials) -> impl FnOnce(&AwsSettings, &str) -> Resolution {
    move |_settings: &AwsSettings, _region: &str| ready(Ok(credentials))
}

fn failing() -> impl FnOnce(&AwsSettings, &str) -> Resolution {
    |_settings: &AwsSettings, _region: &str| ready(Err(CredentialError::MissingCredentials))
}
