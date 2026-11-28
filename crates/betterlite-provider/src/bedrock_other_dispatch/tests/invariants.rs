use std::collections::BTreeMap;

use betterlite_bedrock::embedding_call::EmbeddingEnvironment;
use betterlite_bedrock::embedding_jobs::error::EmbeddingJobsError;
use betterlite_bedrock::transcription::binding::TranscriptionCall;
use secrecy::SecretString;

use super::fixture::{Binding, Fixture, Handler, params, run, transcription_params};
use crate::bedrock_other_dispatch::call::{
    BedrockOtherOperation, EmbeddingInput, EmbeddingOperation, EmbeddingStatusOperation,
};
use crate::bedrock_other_dispatch::error::BedrockOtherError;
use crate::bedrock_other_dispatch::test_support::client::Reply;
use crate::bedrock_other_dispatch::test_support::{NOW_SECONDS, credentials, instant};

const FIRST_ACCESS: &str = "AKIAIOSFODNN7EXAMPLE";
const FIRST_SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const SECOND_ACCESS: &str = "AKIAI44QH8DHBEXAMPLE";
const SECOND_SECRET: &str = "je7MtGbClwBF/2Zp9Utk/h3yCo8nvbEXAMPLEKEY";
const TITAN: &str = "amazon.titan-embed-text-v1";
const REPLY: &str = r#"{"embedding": [0.5], "inputTextTokenCount": 1}"#;
const IDENTITY: &str = r#"{"aws_region_name": "us-east-1"}"#;
const INVOCATION: &str = "arn:aws:bedrock:us-east-1:123456789012:async-invoke/abc";

fn embedding_operation<'call>(
    input: &'call [String],
    optional: &'call str,
    environment: &'call BTreeMap<String, String>,
) -> BedrockOtherOperation<'call, 'call, 'call, Handler, Binding> {
    BedrockOtherOperation::Embedding(EmbeddingOperation {
        model: TITAN,
        input: EmbeddingInput::Multiple(input),
        optional_params: params(optional),
        api_base: None,
        extra_headers: None,
        api_key: None,
        timeout: None,
        aembedding: false,
        drop_params: false,
        environment: EmbeddingEnvironment {
            variables: environment,
            runtime_endpoint: None,
            bearer_token: None,
        },
    })
}

// One Titan embedding signed by the identity given, with the region its source was asked for
async fn signed(access: &str, secret: &str) -> (String, Vec<String>) {
    let environment = BTreeMap::new();
    let input = vec![String::from("hello")];
    let fixture = Fixture::new(&[Reply::status(200, REPLY)], credentials(access, secret));
    let outcome = run(
        &fixture,
        embedding_operation(&input, IDENTITY, &environment),
    )
    .await;
    assert_eq!(outcome.requests.len(), 1, "one identity sends one request");
    let header = outcome
        .authorization()
        .expect("the identity signs its request")
        .to_owned();
    (header, outcome.regions)
}

// Two configured identities never share a signature, and a repeat call keeps its own identity's
#[tokio::test]
async fn bedrock_other_dispatch_credential_identity_isolation() {
    let (first, first_regions) = signed(FIRST_ACCESS, FIRST_SECRET).await;
    let (second, second_regions) = signed(SECOND_ACCESS, SECOND_SECRET).await;
    assert_ne!(first, second);
    assert!(first.contains(FIRST_ACCESS));
    assert!(!first.contains(SECOND_ACCESS));
    assert!(second.contains(SECOND_ACCESS));
    assert!(!second.contains(FIRST_ACCESS));
    assert_eq!(first_regions, [String::from("us-east-1")]);
    assert_eq!(second_regions, [String::from("us-east-1")]);
    let (repeat, repeat_regions) = signed(FIRST_ACCESS, FIRST_SECRET).await;
    assert_eq!(repeat, first, "the same identity signs the same request");
    assert_eq!(repeat_regions, first_regions);
}

// The call the binding seam receives prints the names of its sensitive fields, never their values
fn call_debug_masks_values() {
    let optional = transcription_params(
        r#"{"aws_secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "aws_access_key_id": "AKIAIOSFODNN7EXAMPLE"}"#,
    );
    let headers = transcription_params(r#"{"Authorization": "Bearer top-secret-token"}"#);
    let api_key = SecretString::from(String::from("api-key-secret"));
    let call = TranscriptionCall {
        model: "amazon.nova-2-sonic-v1:0",
        api_key: Some(&api_key),
        api_base: None,
        custom_llm_provider: "bedrock",
        extra_headers: Some(&headers),
        optional_params: &optional,
        timeout: None,
    };
    let printed = format!("{call:?}");
    assert!(printed.contains("aws_secret_access_key"));
    assert!(printed.contains("Authorization"));
    for value in [
        FIRST_ACCESS,
        FIRST_SECRET,
        "top-secret-token",
        "api-key-secret",
    ] {
        assert!(!printed.contains(value), "the call Debug prints no value");
    }
}

// A provider reply that echoes the fixture's credentials is hidden by the composed failure's Debug
async fn status_debug_masks_reply() {
    let echoed =
        format!(r#"{{"message": "the credential {FIRST_ACCESS}/{FIRST_SECRET} was refused"}}"#);
    let fixture = Fixture::new(
        &[Reply::status(403, &echoed)],
        credentials(FIRST_ACCESS, FIRST_SECRET),
    );
    let mut optional = params(IDENTITY);
    let environment = BTreeMap::new();
    let outcome = run(
        &fixture,
        BedrockOtherOperation::EmbeddingStatus(EmbeddingStatusOperation {
            invocation_arn: INVOCATION,
            region: "us-east-1",
            optional_params: &mut optional,
            environment: &environment,
            now: instant(NOW_SECONDS).expect("the fixture instant"),
        }),
    )
    .await;
    let error = outcome.error();
    assert!(matches!(
        error,
        BedrockOtherError::EmbeddingStatus(EmbeddingJobsError::HttpFailure { status: 403, .. })
    ));
    assert_eq!(error.status(), Some(403));
    let printed = format!("{error:?}");
    assert!(printed.contains("[REDACTED provider reply]"));
    assert!(!printed.contains(FIRST_ACCESS));
    assert!(!printed.contains(FIRST_SECRET));
}

// Debug and owned failure paths do not expose fixture credentials
#[tokio::test]
async fn bedrock_other_dispatch_secret_safe_failure() {
    call_debug_masks_values();
    status_debug_masks_reply().await;
}
