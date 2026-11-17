#![cfg(test)]

use std::iter::once;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::converse_response::environment::validate_environment;
use crate::converse_response::transform::TransformError;
use crate::request_policy::{BedrockRequestPolicy, Params, REQUEST_METADATA_HEADER, RequestValue};

use super::support::{convert, fixture};

const SECRET: &str = "sk-fixture-secret-000000000000";

#[test]
fn converse_response_credential_identity_isolation() {
    let caller = map(&[("X-Caller", "kept")]);
    let empty = Params::new();
    let first = validate_environment(&caller, Some(&SecretString::from("key-one")), None, &empty)
        .expect("the headers merge");
    let second = validate_environment(&caller, Some(&SecretString::from("key-two")), None, &empty)
        .expect("the headers merge");
    assert_eq!(
        first.get("Authorization").map(String::as_str),
        Some("Bearer key-one")
    );
    assert_eq!(
        second.get("Authorization").map(String::as_str),
        Some("Bearer key-two")
    );
    assert_ne!(first, second);
    assert_eq!(caller, map(&[("X-Caller", "kept")]));

    // A later call never sees the identity of an earlier one, and the caller's map gains nothing.
    let keyless = validate_environment(&caller, None, None, &empty).expect("the headers merge");
    assert_eq!(keyless, caller);
    assert!(!caller.contains_key("Authorization"));
    metadata_identity_is_isolated();
}

fn metadata_identity_is_isolated() {
    // Each request resolves only its own identity fields, even when the policy is shared.
    let policy = BedrockRequestPolicy {
        fields: once("user_api_key_team_id".to_owned()).collect(),
    };
    let one = metadata_params("team-one");
    let two = metadata_params("team-two");
    let first_metadata = validate_environment(&IndexMap::new(), None, Some(&policy), &one)
        .expect("the headers merge");
    let second_metadata = validate_environment(&IndexMap::new(), None, Some(&policy), &two)
        .expect("the headers merge");
    assert_eq!(
        first_metadata
            .get(REQUEST_METADATA_HEADER)
            .map(String::as_str),
        Some(r#"{"user_api_key_team_id":"team-one"}"#)
    );
    assert_eq!(
        second_metadata
            .get(REQUEST_METADATA_HEADER)
            .map(String::as_str),
        Some(r#"{"user_api_key_team_id":"team-two"}"#)
    );
    assert_ne!(first_metadata, second_metadata);
}

#[test]
fn converse_response_secret_safe_failure() {
    let mut fixture = fixture();
    fixture.headers = vec![
        (String::from("Authorization"), format!("Bearer {SECRET}")),
        (String::from("x-amzn-requestid"), String::from("req-1")),
    ];
    let run = convert(&fixture, "{not json");
    let failure = run.failure();
    let display = format!("{failure}");
    assert!(
        !display.contains(SECRET),
        "the failure names the credential: {display}"
    );
    let debug = format!("{failure:?}");
    assert!(
        !debug.contains(SECRET),
        "the failure names the credential: {debug}"
    );
    let TransformError::Conversion { error, .. } = failure else {
        panic!("an unreadable body is a conversion failure");
    };
    assert!(error.headers.is_some());
    assert!(!format!("{error:?}").contains(SECRET));
    assert_eq!(error.response_headers, fixture.headers);

    // The post-call record carries the request body and additional args, never the credential.
    let recorded = serde_json::to_string(&run.details).expect("the details serialize");
    assert!(
        !recorded.contains(SECRET),
        "the log record names the credential"
    );
    let Json::Object(additional) = run.detail("additional_args") else {
        panic!("the additional args are an object");
    };
    assert!(!additional.contains_key("api_key"));
    assert!(!additional.contains_key("authorization"));
}

fn map(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn metadata_params(team: &str) -> Params {
    let mut metadata = Params::new();
    metadata.insert(
        String::from("user_api_key_team_id"),
        RequestValue::Text(team.to_owned()),
    );
    let mut params = Params::new();
    params.insert(String::from("metadata"), RequestValue::Mapping(metadata));
    params
}
