use std::time::{Duration, SystemTime};

use betterlite_models::decisions::Json;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig;
use betterlite_openai::passthrough::transformation::types::PassthroughSigning;
use secrecy::SecretString;

use super::super::error::{PassthroughFailure, ProviderFailure};
use super::super::event::{LogEventError, decode_passthrough_log_event};
use super::super::reply::LOGGED_MESSAGES;
use super::super::{BedrockPassthroughPlan, PassthroughIdentity};
use super::{credentials, environment, fixtures, identity, json, object, plan};

fn signing_context<'text>(
    litellm_params: &'text Object,
    request_data: Option<&'text Object>,
) -> PassthroughSigning<'text> {
    PassthroughSigning {
        litellm_params,
        request_data,
        api_base: "https://bedrock-runtime.us-east-1.amazonaws.com/model/m/converse",
        model: Some("m"),
    }
}

fn sign(config: &BedrockPassthroughPlan<'_>) -> String {
    let params = object(r#"{"aws_region_name":"us-east-1"}"#);
    config
        .sign_request(Headers::new(), &signing_context(&params, None))
        .unwrap()
        .0
        .get("Authorization")
        .cloned()
        .unwrap()
}

#[test]
fn passthrough_credential_identity_isolation() {
    let (selector, transform, assembler) = fixtures();
    let first = plan(
        &selector,
        &transform,
        &assembler,
        environment(),
        PassthroughIdentity {
            credentials: Some(credentials("AKIAFIRST", "first-secret")),
            bearer_token: None,
            now: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        },
    );
    let second = plan(
        &selector,
        &transform,
        &assembler,
        environment(),
        PassthroughIdentity {
            credentials: Some(credentials("AKIASECOND", "second-secret")),
            bearer_token: None,
            now: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        },
    );
    let first_authorization = sign(&first);
    let second_authorization = sign(&second);
    assert!(first_authorization.contains("Credential=AKIAFIRST/"));
    assert!(second_authorization.contains("Credential=AKIASECOND/"));
    assert_ne!(first_authorization, second_authorization);
    assert!(!first_authorization.contains("AKIASECOND"));
    assert!(!second_authorization.contains("AKIAFIRST"));
    let format = format!("{first:?}");
    assert!(format.contains("has_credentials: true"));
    assert!(!format.contains("first-secret"));
    let mut headers = Headers::new();
    headers.insert("x-first".to_owned(), "1".to_owned());
    let params = object(r#"{"aws_region_name":"us-east-1"}"#);
    let (mut signed, _) = first
        .sign_request(headers.clone(), &signing_context(&params, None))
        .unwrap();
    signed.insert("x-first".to_owned(), "changed".to_owned());
    let (untouched, _) = second
        .sign_request(Headers::new(), &signing_context(&params, None))
        .unwrap();
    assert!(!untouched.contains_key("x-first"));
    assert_eq!(headers.get("x-first").map(String::as_str), Some("1"));
    assert_eq!(first.environment, second.environment);
}

#[test]
fn passthrough_secret_safe_failure() {
    let (selector, transform, assembler) = fixtures();
    let config = plan(
        &selector,
        &transform,
        &assembler,
        environment(),
        PassthroughIdentity {
            credentials: Some(credentials("AKIAFIXTURE", "FIXTURE-SECRET")),
            bearer_token: Some(SecretString::from("FIXTURE-BEARER".to_owned())),
            now: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        },
    );
    let format = format!("{config:?}");
    assert!(format.contains("has_credentials: true"));
    assert!(format.contains("has_bearer_token: true"));
    assert!(!format.contains("FIXTURE-SECRET"));
    assert!(!format.contains("FIXTURE-BEARER"));
    let sigv4_config = plan(&selector, &transform, &assembler, environment(), identity());
    let params = object(r#"{"aws_region_name":"BAD_REGION"}"#);
    let failure = sigv4_config
        .sign_request(Headers::new(), &signing_context(&params, None))
        .unwrap_err();
    assert!(!format!("{failure:?}").contains("FIXTURE-SECRET"));
    assert!(!failure.to_string().contains("FIXTURE-SECRET"));
    assert!(!failure.to_string().contains("FIXTURE-BEARER"));
    let provider = ProviderError::from(ProviderErrorParts {
        status_code: 500,
        message: "the credential FIXTURE-SECRET leaked".to_owned(),
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    });
    let hidden = PassthroughFailure::Reply(ProviderFailure(provider));
    assert!(!format!("{hidden:?}").contains("FIXTURE-SECRET"));
    assert!(!hidden.to_string().contains("FIXTURE-SECRET"));
    assert!(format!("{hidden:?}").contains("REDACTED"));
    assert_eq!(hidden.to_string(), "the provider request failed");
    assert_eq!(LOGGED_MESSAGES.len(), 1);
    assert_eq!(LOGGED_MESSAGES[0].role, "user");
    assert_eq!(
        LOGGED_MESSAGES[0].content,
        "no-message-pass-through-endpoint"
    );
    let mut headers = Headers::new();
    headers.insert(
        "authorization".to_owned(),
        "Bearer FIXTURE-BEARER".to_owned(),
    );
    let bearer_params = object(r#"{"aws_region_name":"us-east-1"}"#);
    let (bearer_headers, _) = config
        .sign_request(headers, &signing_context(&bearer_params, None))
        .unwrap();
    assert_eq!(
        bearer_headers.get("authorization").map(String::as_str),
        Some("Bearer FIXTURE-BEARER")
    );
    assert_eq!(
        bearer_headers.get("Authorization").map(String::as_str),
        Some("Bearer FIXTURE-BEARER")
    );
}

fn outcome(bytes: &[u8]) -> &'static str {
    match decode_passthrough_log_event(bytes) {
        Ok(_) => "ok",
        Err(LogEventError::NotUtf8(_)) => "not-utf8",
        Err(LogEventError::NotJson(_)) => "not-json",
        Err(LogEventError::NotObject) => "not-object",
    }
}

#[test]
fn replays_the_bedrock_services_passthrough_corpus() {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/bedrock_services_passthrough");
    let mut seen = 0_usize;
    for entry in std::fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let bytes = std::fs::read(&path).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        let category = name.splitn(3, '-').nth(2).unwrap();
        assert_eq!(outcome(&bytes), category, "{name}");
        seen += 1;
    }
    assert!(seen >= 6);
    assert_eq!(outcome(br#"{"bytes":"x"}"#), "ok");
    assert_eq!(outcome(br#"  ["not", "an", "object"]"#), "not-object");
    assert_eq!(outcome(b"not json"), "not-json");
    assert_eq!(outcome(b"{\"a\":\"\xff\"}"), "not-utf8");
    let value: Json = json(r#"{"bytes":"x"}"#);
    assert_eq!(
        decode_passthrough_log_event(br#"{"bytes":"x"}"#).unwrap(),
        value
    );
}
