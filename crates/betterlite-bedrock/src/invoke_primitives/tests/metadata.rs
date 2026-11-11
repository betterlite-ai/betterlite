use indexmap::IndexMap;

use crate::invoke_primitives::environment::{EnvironmentError, error_class, validate_environment};
use crate::request_policy::BedrockRequestPolicy;

use super::support::{headers, mapping, metadata, params, text};

const METADATA_HEADER: &str = "X-Amzn-Bedrock-Request-Metadata";

fn policy(fields: &[&str]) -> BedrockRequestPolicy {
    BedrockRequestPolicy {
        fields: fields.iter().map(|field| (*field).to_owned()).collect(),
    }
}

pub(super) fn guardrail_metadata_merge() {
    caller_header_wins();
    absent_guardrail();
    invalid_guardrail();
    owned_metadata();
    empty_owner();
    error_projection();
}

fn caller_header_wins() {
    let caller = headers(&[
        ("x-amzn-bedrock-guardrailidentifier", "caller"),
        ("x-custom", "keep"),
    ]);
    let mut optional = params(
        r#"{"guardrailConfig":{"guardrailIdentifier":"derived","trace":"disabled"},"temperature":0.2}"#,
    );
    let merged = validate_environment(&caller, &mut optional, &IndexMap::new(), None).unwrap();
    assert_eq!(
        merged
            .get("x-amzn-bedrock-guardrailidentifier")
            .map(String::as_str),
        Some("caller")
    );
    assert_eq!(
        merged.get("X-Amzn-Bedrock-Trace").map(String::as_str),
        Some("DISABLED")
    );
    assert_eq!(merged.get("x-custom").map(String::as_str), Some("keep"));
    assert!(!merged.contains_key("X-Amzn-Bedrock-GuardrailIdentifier"));
    // the guardrail configuration is popped and the rest of the options stay
    assert!(!optional.contains_key("guardrailConfig"));
    assert!(optional.contains_key("temperature"));
}

fn absent_guardrail() {
    let present = headers(&[("x-only", "v")]);
    for absent in [r#"{"temperature":0.2}"#, r#"{"guardrailConfig":null}"#] {
        let mut optional = params(absent);
        let merged = validate_environment(&present, &mut optional, &IndexMap::new(), None).unwrap();
        assert_eq!(merged, present, "{absent}");
    }
}

fn invalid_guardrail() {
    let mut optional = params(r#"{"guardrailConfig":{"guardrailIdentifier":5}}"#);
    let failure =
        validate_environment(&headers(&[]), &mut optional, &IndexMap::new(), None).unwrap_err();
    match failure {
        EnvironmentError::Guardrail(guardrail) => assert_eq!(guardrail.status_code(), 400),
        EnvironmentError::GuardrailJson { .. }
        | EnvironmentError::Metadata(_)
        | EnvironmentError::Headers(_) => panic!("unexpected environment failure"),
    }
}

fn owned_metadata() {
    let owner = policy(&["user_api_key_hash", "spend_logs_metadata"]);
    let tenant = metadata(&[
        ("user_api_key_hash", text("hash")),
        (
            "spend_logs_metadata",
            mapping(&[("team", text("a")), ("bad key!", text("x"))]),
        ),
    ]);
    let spoofed = headers(&[(METADATA_HEADER, "spoof"), ("x-other", "v")]);
    let merged = validate_environment(&spoofed, &mut params("{}"), &tenant, Some(&owner)).unwrap();
    assert_eq!(
        merged.get(METADATA_HEADER).map(String::as_str),
        Some(r#"{"user_api_key_hash":"hash","team":"a"}"#)
    );
    assert_eq!(merged.get("x-other").map(String::as_str), Some("v"));
    assert!(!merged.contains_key("Authorization"));
}

fn empty_owner() {
    let owner = policy(&["user_api_key_hash"]);
    let spoofed = headers(&[(METADATA_HEADER, "spoof"), ("x-other", "v")]);
    let merged =
        validate_environment(&spoofed, &mut params("{}"), &IndexMap::new(), Some(&owner)).unwrap();
    assert!(!merged.contains_key(METADATA_HEADER));
    assert_eq!(merged.get("x-other").map(String::as_str), Some("v"));
}

fn error_projection() {
    let projected = error_class(
        "boom".to_owned(),
        429,
        vec![("retry-after".to_owned(), "1".to_owned())],
    )
    .unwrap();
    assert_eq!(projected.error.status_code, 429);
    assert_eq!(projected.error.message, "boom");
    assert_eq!(
        projected.error.headers,
        Some(vec![("retry-after".to_owned(), "1".to_owned())])
    );
    assert_eq!(projected.error.request.url, "https://docs.litellm.ai/docs");
}
