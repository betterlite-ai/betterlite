#![cfg(test)]

mod support;

use std::fmt::Debug;
use std::time::Duration;

use betterlite_core::api::vector_store_http_write::error::BaseLlmException;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http_fixture::reply::reply;
use betterlite_openai::watsonx_audio::error::WatsonxAudioError;
use rstest::rstest;
use secrecy::SecretString;

use self::support::{
    AUDIO, Iam, MODEL, Run, SUPPLIED_BODY_BYTES, environment, object, probe_supported_options,
};
use crate::api::transcription_http::error::TranscriptionHttpError;
use crate::watsonx_transcription::error::WatsonxTranscriptionError;

const TRANSCRIPT: &str = r#"{"text":"hello","usage":{"total_tokens":3}}"#;
const DEFAULT_VERSION_POST: &str = "POST /ml/v1/audio/transcriptions?version=2024-03-13 HTTP/1.1";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    TokenWithoutKey,
    ProjectBeforeSpace,
    LegacyProjectName,
    SpaceOnly,
    NoProject,
    ExplicitVersion,
    ResponseExtraFields,
    ProviderBadRequest,
    ResponseBodyLimit,
    RequestDeadline,
    SuppliedClient,
    ProviderNotText,
    SupportedOptions,
}

#[rstest]
#[case::token_without_key(Case::TokenWithoutKey)]
#[case::project_before_space(Case::ProjectBeforeSpace)]
#[case::legacy_project_name(Case::LegacyProjectName)]
#[case::space_only(Case::SpaceOnly)]
#[case::no_project(Case::NoProject)]
#[case::explicit_version(Case::ExplicitVersion)]
#[case::response_extra_fields(Case::ResponseExtraFields)]
#[case::provider_bad_request(Case::ProviderBadRequest)]
#[case::response_body_limit(Case::ResponseBodyLimit)]
#[case::request_deadline(Case::RequestDeadline)]
#[case::supplied_client(Case::SuppliedClient)]
#[case::provider_not_text(Case::ProviderNotText)]
#[case::supported_options(Case::SupportedOptions)]
#[tokio::test]
async fn watsonx_transcription_binding(#[case] case: Case) {
    match case {
        Case::TokenWithoutKey => token_without_key().await,
        Case::ProjectBeforeSpace => project_before_space().await,
        Case::LegacyProjectName => legacy_project_name().await,
        Case::SpaceOnly => space_only().await,
        Case::NoProject => no_project().await,
        Case::ExplicitVersion => explicit_version().await,
        Case::ResponseExtraFields => response_extra_fields().await,
        Case::ProviderBadRequest => provider_bad_request().await,
        Case::ResponseBodyLimit => response_body_limit().await,
        Case::RequestDeadline => request_deadline().await,
        Case::SuppliedClient => supplied_client().await,
        Case::ProviderNotText => provider_not_text().await,
        Case::SupportedOptions => supported_options(),
    }
}

fn json_reply(body: &str) -> Vec<u8> {
    reply("200 OK", &[("content-type", "application/json")], body)
}

fn provider_error<T>(result: Result<T, WatsonxTranscriptionError>) -> BaseLlmException
where
    T: Debug,
{
    let WatsonxTranscriptionError::Http(TranscriptionHttpError::Provider(provider)) =
        result.expect_err("the call fails")
    else {
        panic!("the executor failure is a provider error");
    };
    provider
}

// The auth that survives the multipart rewrite: Bearer, JSON accepted, and no JSON content type left
fn assert_multipart_head(head: &str) {
    assert!(head.contains("authorization: Bearer envtoken"));
    assert!(head.contains("accept: application/json"));
    assert!(!head.contains("content-type: application/json"));
    assert!(head.contains("content-type: multipart/form-data; boundary="));
}

// httpx names a plain value's part upload, and the form names the model field model, not model_id
fn assert_multipart_body(body: &str) {
    assert!(body.contains("name=\"file\"; filename=\"upload\""));
    assert!(body.contains("Content-Type: application/octet-stream"));
    assert!(body.contains(AUDIO));
    assert!(body.contains("name=\"model\""));
    assert!(body.contains(MODEL));
    assert!(!body.contains("model_id"));
}

async fn token_without_key() {
    let run = Run::new(json_reply(TRANSCRIPT));
    let outcome = run
        .execute(
            environment(&[("WATSONX_TOKEN", "envtoken")]),
            Iam::refusing(),
        )
        .await;
    let transcription = outcome.result.expect("a token needs no API key");
    assert_eq!(transcription.response.text.as_deref(), Some("hello"));
    assert_eq!(
        outcome.iam_calls, 0,
        "a token never performs the IAM exchange"
    );
    assert_eq!(
        outcome.requests.len(),
        1,
        "the call sends exactly one request"
    );
    let request = &outcome.requests[0];
    assert_eq!(request.line, DEFAULT_VERSION_POST);
    assert_multipart_head(request.head.as_str());
    assert_multipart_body(request.body.as_str());
}

async fn project_before_space() {
    let mut run = Run::new(json_reply(TRANSCRIPT));
    run.optional = object(
        r#"{"project_id":"p","space_id":"s","language":"en","prompt":"hi","response_format":"json","temperature":0.2,"timestamp_granularities":["word"]}"#,
    );
    let outcome = run
        .execute(
            environment(&[("WATSONX_TOKEN", "envtoken")]),
            Iam::refusing(),
        )
        .await;
    outcome.result.expect("the form call succeeds");
    assert_eq!(outcome.requests.len(), 1, "the form call sends once");
    let body = &outcome.requests[0].body;
    assert!(body.contains("name=\"project_id\"\r\n\r\np"));
    assert!(!body.contains("name=\"space_id\""), "the project wins");
    for name in [
        "language",
        "prompt",
        "response_format",
        "temperature",
        "timestamp_granularities",
    ] {
        assert!(
            body.contains(&format!("name=\"{name}\"")),
            "the form carries {name}"
        );
    }
    let model = body
        .find("name=\"model\"")
        .expect("the form names the model");
    let project = body
        .find("name=\"project_id\"")
        .expect("the form names the project");
    assert!(model < project, "the model is written before the project");
}

async fn legacy_project_name() {
    let mut run = Run::new(json_reply(TRANSCRIPT));
    run.optional = object(r#"{"watsonx_project":"wp","space_id":"s"}"#);
    let outcome = run
        .execute(
            environment(&[("WATSONX_TOKEN", "envtoken")]),
            Iam::refusing(),
        )
        .await;
    outcome.result.expect("the legacy project name is accepted");
    assert_eq!(outcome.requests.len(), 1, "the legacy call sends once");
    let body = &outcome.requests[0].body;
    assert!(body.contains("name=\"project_id\"\r\n\r\nwp"));
    assert!(!body.contains("name=\"space_id\""), "the project wins");
}

async fn space_only() {
    let mut run = Run::new(json_reply(TRANSCRIPT));
    run.optional = object(r#"{"space_id":"s"}"#);
    let outcome = run
        .execute(
            environment(&[("WATSONX_TOKEN", "envtoken")]),
            Iam::refusing(),
        )
        .await;
    outcome.result.expect("a space-only call succeeds");
    assert_eq!(outcome.requests.len(), 1, "the space call sends once");
    let body = &outcome.requests[0].body;
    assert!(body.contains("name=\"space_id\"\r\n\r\ns"));
    assert!(!body.contains("name=\"project_id\""));
}

async fn no_project() {
    let run = Run::new(json_reply(TRANSCRIPT));
    let outcome = run
        .execute(
            environment(&[
                ("WATSONX_TOKEN", "envtoken"),
                ("WATSONX_PROJECT_ID", "env-project"),
                ("PROJECT_ID", "env-project-2"),
                ("WATSONX_SPACE_ID", "env-space"),
            ]),
            Iam::refusing(),
        )
        .await;
    outcome.result.expect("both ids may be absent");
    assert_eq!(outcome.requests.len(), 1, "the id-less call sends once");
    let body = &outcome.requests[0].body;
    assert!(
        !body.contains("name=\"project_id\""),
        "no project from the environment"
    );
    assert!(
        !body.contains("name=\"space_id\""),
        "no space from the environment"
    );
}

async fn explicit_version() {
    let mut run = Run::new(json_reply(TRANSCRIPT));
    run.optional = object(r#"{"api_version":"2024-11-01"}"#);
    run.api_key = Some(SecretString::from("sk-watsonx".to_owned()));
    let outcome = run
        .execute(
            environment(&[("WATSONX_IAM_URL", "https://iam.example/identity/token")]),
            Iam::answering(r#"{"access_token":"iam-token","expires_in":3600}"#),
        )
        .await;
    outcome.result.expect("the keyed IAM call succeeds");
    assert_eq!(
        outcome.iam_calls, 1,
        "the explicit key performs one exchange"
    );
    assert_eq!(outcome.requests.len(), 1, "the keyed call sends once");
    assert_eq!(
        outcome.requests[0].line,
        "POST /ml/v1/audio/transcriptions?version=2024-11-01 HTTP/1.1"
    );
    assert!(
        outcome.requests[0]
            .head
            .contains("authorization: Bearer iam-token")
    );
}

async fn response_extra_fields() {
    let run = Run::new(json_reply(
        r#"{"text":"hello","usage":{"total_tokens":3},"model":"granite","duration":2.5,"segments":[{"id":0,"start":0.0,"end":1.0,"text":"hi"}]}"#,
    ));
    let outcome = run
        .execute(
            environment(&[("WATSONX_TOKEN", "envtoken")]),
            Iam::refusing(),
        )
        .await;
    let transcription = outcome.result.expect("the reply is accepted");
    assert_eq!(transcription.response.text.as_deref(), Some("hello"));
    let expected = object(
        r#"{"usage":{"total_tokens":3},"duration":2.5,"segments":[{"id":0,"start":0.0,"end":1.0,"text":"hi"}]}"#,
    );
    for name in ["usage", "duration", "segments"] {
        assert_eq!(
            transcription.response.extra.get(name),
            expected.get(name),
            "the reply keeps {name}"
        );
    }
    assert!(
        !transcription.response.extra.contains_key("model"),
        "the model field is discarded"
    );
}

async fn provider_bad_request() {
    let body = r#"{"errors":[{"code":"bad_request","message":"model is required"}]}"#;
    let run = Run::new(reply(
        "400 Bad Request",
        &[("content-type", "application/json")],
        body,
    ));
    let outcome = run
        .execute(
            environment(&[("WATSONX_TOKEN", "envtoken")]),
            Iam::refusing(),
        )
        .await;
    let provider = provider_error(outcome.result);
    assert_eq!(provider.status_code, 400);
    assert!(!provider.status_code_is_synthesized);
    assert!(provider.message.contains("model is required"));
    assert_eq!(outcome.requests.len(), 1, "a refusal is not retried");
}

async fn response_body_limit() {
    let mut run = Run::new(json_reply(TRANSCRIPT));
    run.limits = TransportLimits {
        body_bytes: 16,
        ..TransportLimits::DEFAULT
    };
    let outcome = run
        .execute(
            environment(&[("WATSONX_TOKEN", "envtoken")]),
            Iam::refusing(),
        )
        .await;
    let provider = provider_error(outcome.result);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(
        provider.message,
        "the response body is larger than 16 bytes"
    );
}

async fn request_deadline() {
    let mut run = Run::new(json_reply(TRANSCRIPT));
    run.asynchronous = true;
    run.timeout = Some(Duration::from_millis(250));
    run.pause = Duration::from_millis(600);
    let outcome = run
        .execute(
            environment(&[("WATSONX_TOKEN", "envtoken")]),
            Iam::refusing(),
        )
        .await;
    let provider = provider_error(outcome.result);
    assert_eq!(provider.status_code, 408);
    assert!(
        provider
            .message
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=0.25"),
        "the async deadline names the timeout: {}",
        provider.message
    );
    assert_eq!(
        outcome.requests.len(),
        1,
        "the request reached the provider"
    );
}

async fn supplied_client() {
    let mut run = Run::new(json_reply(TRANSCRIPT));
    run.supplied = true;
    let outcome = run
        .execute(
            environment(&[("WATSONX_TOKEN", "envtoken")]),
            Iam::refusing(),
        )
        .await;
    // The supplied client reads with its own small body limit; the pooled client would accept the reply
    let provider = provider_error(outcome.result);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(
        provider.message,
        format!("the response body is larger than {SUPPLIED_BODY_BYTES} bytes")
    );
    assert_eq!(
        outcome.requests.len(),
        1,
        "the supplied client sent the call"
    );
}

async fn provider_not_text() {
    let mut run = Run::new(json_reply(TRANSCRIPT));
    run.optional = object(r#"{"space_id":7}"#);
    let outcome = run
        .execute(
            environment(&[("WATSONX_TOKEN", "envtoken")]),
            Iam::refusing(),
        )
        .await;
    let WatsonxTranscriptionError::Audio(WatsonxAudioError::NotText { field }) =
        outcome.result.expect_err("a number is not a space id")
    else {
        panic!("the adapter keeps B5842's error class");
    };
    assert_eq!(field, "space_id");
    assert!(outcome.requests.is_empty(), "nothing reaches the provider");
}

fn supported_options() {
    let supported = probe_supported_options(environment(&[("WATSONX_TOKEN", "envtoken")]));
    let expected = [
        "language",
        "prompt",
        "response_format",
        "temperature",
        "timestamp_granularities",
    ]
    .map(String::from)
    .to_vec();
    assert_eq!(supported, expected);
}
