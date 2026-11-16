use std::fs;
use std::path::PathBuf;

use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use rstest::rstest;
use secrecy::SecretString;

use super::super::error::PassthroughFailure;
use super::super::event::{LogEventError, decode_passthrough_log_event};
use super::super::{BedrockPassthroughPlan, PassthroughIdentity};
use super::{
    credentials, environment, fixture_time, fixtures, identity, json, object, plan, signing_context,
};

fn identity_with(access: &str, secret: &str) -> PassthroughIdentity {
    PassthroughIdentity {
        credentials: Some(credentials(access, secret)),
        bearer_token: None,
        now: fixture_time(),
    }
}

fn sign(config: &BedrockPassthroughPlan<'_>, headers: Headers) -> Headers {
    let params = object(r#"{"aws_region_name":"us-east-1"}"#);
    let signed = config.sign_request(headers, &signing_context(&params, None));
    signed.unwrap().0
}

fn headers_stay_with_their_plan(
    first: &BedrockPassthroughPlan<'_>,
    second: &BedrockPassthroughPlan<'_>,
) {
    let mut headers = Headers::new();
    headers.insert("x-first".to_owned(), "1".to_owned());
    let signed = sign(first, headers.clone());
    assert_eq!(signed.get("x-first").map(String::as_str), Some("1"));
    assert!(!sign(second, Headers::new()).contains_key("x-first"));
    assert_eq!(headers.get("x-first").map(String::as_str), Some("1"));
    assert_eq!(first.environment, second.environment);
}

#[test]
fn passthrough_credential_identity_isolation() {
    let (selector, transform, assembler) = fixtures();
    let first = plan(
        &selector,
        &transform,
        &assembler,
        environment(),
        identity_with("AKIAFIRST", "first-secret"),
    );
    let second = plan(
        &selector,
        &transform,
        &assembler,
        environment(),
        identity_with("AKIASECOND", "second-secret"),
    );
    let first_signature = sign(&first, Headers::new())["Authorization"].clone();
    let second_signature = sign(&second, Headers::new())["Authorization"].clone();
    assert!(first_signature.contains("Credential=AKIAFIRST/"));
    assert!(second_signature.contains("Credential=AKIASECOND/"));
    assert_ne!(first_signature, second_signature);
    assert!(!first_signature.contains("AKIASECOND"));
    assert!(!second_signature.contains("AKIAFIRST"));
    headers_stay_with_their_plan(&first, &second);
}

fn plan_debug_hides_secrets() {
    let (selector, transform, assembler) = fixtures();
    let mut configured = identity_with("AKIAFIXTURE", "FIXTURE-SECRET");
    configured.bearer_token = Some(SecretString::from("FIXTURE-BEARER".to_owned()));
    let config = plan(&selector, &transform, &assembler, environment(), configured);
    let debug = format!("{config:?}");
    assert!(debug.contains("has_credentials: true"));
    assert!(debug.contains("has_bearer_token: true"));
    assert!(!debug.contains("FIXTURE-SECRET"));
    assert!(!debug.contains("FIXTURE-BEARER"));
}

fn signing_failure_hides_secrets() {
    let (selector, transform, assembler) = fixtures();
    let config = plan(&selector, &transform, &assembler, environment(), identity());
    let params = object(r#"{"aws_region_name":"BAD_REGION"}"#);
    let failure = config
        .sign_request(Headers::new(), &signing_context(&params, None))
        .unwrap_err();
    assert!(!format!("{failure:?}").contains("fixture-secret"));
    assert!(!failure.to_string().contains("fixture-secret"));
}

fn provider_failure_hides_the_message() {
    let provider = ProviderError::from(ProviderErrorParts {
        status_code: 500,
        message: "the credential FIXTURE-SECRET leaked".to_owned(),
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    });
    let hidden = PassthroughFailure::from(provider);
    assert!(!format!("{hidden:?}").contains("FIXTURE-SECRET"));
    assert!(format!("{hidden:?}").contains("REDACTED"));
    assert_eq!(hidden.to_string(), "the provider request failed");
}

#[test]
fn passthrough_secret_safe_failure() {
    plan_debug_hides_secrets();
    signing_failure_hides_secrets();
    provider_failure_hides_the_message();
}

fn outcome(bytes: &[u8]) -> &'static str {
    match decode_passthrough_log_event(bytes) {
        Ok(_) => "ok",
        Err(LogEventError::NotUtf8(_)) => "not-utf8",
        Err(LogEventError::NotJson(_)) => "not-json",
        Err(LogEventError::NotObject) => "not-object",
    }
}

#[rstest]
fn replays_the_bedrock_services_passthrough_corpus(
    #[files("fuzz/corpus/bedrock_services_passthrough/*")] path: PathBuf,
) {
    let name = path.file_name().unwrap().to_str().unwrap();
    let category = name.splitn(3, '-').nth(2).unwrap();
    assert_eq!(outcome(&fs::read(&path).unwrap()), category, "{name}");
}

#[test]
fn log_event_boundaries_keep_their_category() {
    assert_eq!(outcome(br#"  ["not", "an", "object"]"#), "not-object");
    assert_eq!(outcome(b"{\"a\":\"\xff\"}"), "not-utf8");
    let value = json(r#"{"bytes":"x"}"#);
    assert_eq!(
        decode_passthrough_log_event(br#"{"bytes":"x"}"#).unwrap(),
        value
    );
}
