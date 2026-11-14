use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_openai::api::audio_file_input::AudioFile;
use secrecy::SecretString;

use super::support::{NoFiles, env, params};
use crate::transcription::auth::{
    CredentialSource, ValidatedEnvironment, credential_source, sign_transcription,
    validate_environment,
};
use crate::transcription::binding::{TranscriptionCall, prepare_arguments};
use crate::transcription::error::TranscriptionError;

const URL: &str = "https://bedrock-runtime.us-east-1.amazonaws.com/model/m/converse";
const BODY: &str = r#"{"messages":[]}"#;
const HOSTED_ONE: &str = r#"{"aws_access_key_id":"AKIDONE","aws_secret_access_key":"secret-one-value","aws_session_token":"token-one-value"}"#;
const HOSTED_TWO: &str =
    r#"{"aws_access_key_id":"AKIDTWO","aws_secret_access_key":"secret-two-value"}"#;
const CHAIN_ONE: &str = r#"{"aws_access_key_id":"AKIDONE","aws_session_token":"token-one-value"}"#;
const SECRETS: [&str; 5] = [
    "AKIDONE",
    "secret-one-value",
    "token-one-value",
    "api-key-value",
    "ambient-secret",
];

fn ambient() -> BTreeMap<String, String> {
    env(&[
        ("AWS_ACCESS_KEY_ID", "ambient-key"),
        ("AWS_SECRET_ACCESS_KEY", "ambient-secret"),
        ("AWS_ROLE_NAME", "ambient-role"),
    ])
}

fn environment(source: &str) -> ValidatedEnvironment {
    validate_environment(vec![], "bedrock/us-east-1/m", &params(source), &ambient())
}

fn signed_headers(source: &str) -> Vec<(String, String)> {
    let validated = environment(source);
    let CredentialSource::HostSupplied(credentials) = &validated.credentials else {
        panic!("host supplied credentials expected: {source}");
    };
    let now = UNIX_EPOCH + Duration::from_secs(1_440_000_000);
    sign_transcription(&validated, URL, BODY, credentials, now)
        .unwrap()
        .headers
}

fn header<'headers>(headers: &'headers [(String, String)], name: &str) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn chain(source: &str) -> AwsSettings {
    match credential_source(&params(source), &ambient()) {
        CredentialSource::Chain(settings) => *settings,
        CredentialSource::HostSupplied(_) => panic!("chain expected: {source}"),
    }
}

#[test]
fn transcription_credential_identity_isolation() {
    let one = signed_headers(HOSTED_ONE);
    let two = signed_headers(HOSTED_TWO);
    let scope = "20150819/us-east-1/bedrock/aws4_request";
    let auth_one = header(&one, "authorization").unwrap();
    let auth_two = header(&two, "authorization").unwrap();
    assert!(auth_one.contains(&format!("Credential=AKIDONE/{scope}")));
    assert!(auth_two.contains(&format!("Credential=AKIDTWO/{scope}")));
    assert_ne!(auth_one, auth_two);
    assert!(!auth_one.contains("ambient-key"));
    assert!(!auth_two.contains("ambient-key"));
    assert_eq!(
        header(&one, "x-amz-security-token"),
        Some("token-one-value")
    );
    assert_eq!(header(&two, "x-amz-security-token"), None);
    assert_eq!(signed_headers(HOSTED_ONE), one);
    let role_a = chain(r#"{"aws_role_name":"role-a"}"#);
    let role_b = chain(r#"{"aws_role_name":"role-b"}"#);
    let ambient_only = chain("{}");
    assert_eq!(role_a.role_name.as_deref(), Some("role-a"));
    assert_eq!(role_b.role_name.as_deref(), Some("role-b"));
    assert_eq!(ambient_only.role_name.as_deref(), Some("ambient-role"));
    assert!(role_a.access_key_id.is_some() && role_b.access_key_id.is_some());
}

#[test]
fn transcription_secret_safe_failure() {
    let validated = environment(HOSTED_ONE);
    let CredentialSource::HostSupplied(credentials) = &validated.credentials else {
        panic!("host supplied credentials expected");
    };
    let failure =
        sign_transcription(&validated, "not a url", BODY, credentials, UNIX_EPOCH).unwrap_err();
    assert!(matches!(failure, TranscriptionError::Signing(_)));
    let key = SecretString::from("api-key-value".to_owned());
    let options = params(HOSTED_ONE);
    let call = TranscriptionCall {
        model: "bedrock/m",
        api_key: Some(&key),
        api_base: None,
        custom_llm_provider: "bedrock",
        extra_headers: None,
        optional_params: &options,
        timeout: None,
    };
    let arguments = prepare_arguments(AudioFile::Bytes(b"RIFF"), &NoFiles, &call).unwrap();
    let rendered = format!(
        "{failure} {failure:?} {validated:?} {call:?} {arguments:?} {:?}",
        chain(CHAIN_ONE)
    );
    for secret in SECRETS {
        assert!(!rendered.contains(secret), "{secret}");
    }
    let call_text = format!("{call:?}");
    assert!(call_text.contains("TranscriptionCall"), "{call_text}");
    assert!(call_text.contains("aws_access_key_id"), "{call_text}");
}
