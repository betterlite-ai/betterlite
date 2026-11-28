#![cfg(test)]

use crate::model_routes::ParamValue;
use betterlite_core::api::vector_store_http_write::service::ClientKind;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::controls::control_options::ControlOptions;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use std::time::SystemTime;

use super::ConverseCall;
use super::target::{append_endpoint, converse_model_id};
use crate::converse_request::{ConverseMessageSettings, ConverseSettings};
use crate::converse_response::wire::ModelResponse;

#[test]
fn model_id_keeps_the_wrapper_without_a_region() {
    let model = converse_model_id("bedrock/converse/anthropic.claude-v2", None);
    assert_eq!(model.model_id, "bedrock%2Fconverse%2Fanthropic.claude-v2");
    assert_eq!(model.region, None);
}

#[test]
fn model_id_extracts_region_and_removes_nova_prefix() {
    let model = converse_model_id("bedrock/us-east-1/nova/model", None);
    assert_eq!(model.region.as_deref(), Some("us-east-1"));
    assert_eq!(model.model_id, "model");
}

#[test]
fn explicit_model_id_is_encoded_without_region_extraction() {
    let model = converse_model_id("bedrock/us-east-1/ignored", Some("model/id"));
    assert_eq!(model.model_id, "model%2Fid");
    assert_eq!(model.region, None);
}

#[test]
fn endpoint_suffix_preserves_trailing_slashes() {
    assert_eq!(
        append_endpoint("https://example.test/", "id", false),
        "https://example.test//model/id/converse"
    );
    assert_eq!(
        append_endpoint("https://example.test/", "id", true),
        "https://example.test//model/id/converse-stream"
    );
}

#[test]
fn bedrock_converse_call_secret_safe_failure() {
    let api_key = SecretString::from("fixture-secret-key".to_owned());
    let bearer = SecretString::from("fixture-bearer".to_owned());
    let call = call(&api_key, &bearer);
    let debug = format!("{call:?}");
    assert!(!debug.contains(api_key.expose_secret()));
    assert!(!debug.contains(bearer.expose_secret()));
    assert!(debug.contains("ConverseCall"));
    let mut input: fixture::Input =
        serde_json::from_str(include_str!("tests/fixtures/buffered-response.json")).unwrap();
    input.native.failure = fixture::input::Failure::Timeout;
    let failure = fixture::run(&input).unwrap();
    let shown = failure.error.unwrap();
    assert!(shown.contains("408"));
    assert!(!shown.contains("fixture-bearer"));
}

pub(super) fn call<'call>(
    api_key: &'call SecretString,
    bearer: &'call SecretString,
) -> ConverseCall<'call> {
    ConverseCall {
        model: "bedrock/converse/anthropic.claude-v2",
        messages: &[],
        optional_params: IndexMap::default(),
        litellm_params: IndexMap::default(),
        extra_headers: None,
        api_base: None,
        api_key: Some(api_key),
        timeout: None,
        acompletion: false,
        control: ControlOptions::default(),
        skip_pre_call_logging: false,
        model_response: ModelResponse {
            id: "response-id".to_owned(),
            created: 0,
            model: "model".to_owned(),
            object: "chat.completion".to_owned(),
            system_fingerprint: None,
            choices: Vec::new(),
            usage: None,
            service_tier: None,
            trace: None,
        },
        settings: ConverseSettings {
            global_drop: false,
            messages: ConverseMessageSettings {
                modify_params: false,
                neutralize_orphans: true,
            },
            request_metadata: None,
        },
        environment: super::ConverseEnvironment {
            bearer_token: Some(bearer),
            region_name: None,
            region: None,
            session_region: None,
            runtime_endpoint: None,
        },
        now: SystemTime::UNIX_EPOCH,
    }
}

use crate as bedrock;
#[path = "tests/fixtures/mod.rs"]
mod fixture;

#[rstest::rstest]
#[case::converse_signed_path(include_str!("tests/fixtures/converse-signed-path.json"), "signed")]
#[case::converse_stream_path(include_str!("tests/fixtures/converse-stream-path.json"), "fake")]
#[case::bearer_converse(include_str!("tests/fixtures/bearer-converse.json"), "bearer")]
#[case::buffered_response(include_str!("tests/fixtures/buffered-response.json"), "buffered")]
#[case::streaming_first_chunk(include_str!("tests/fixtures/streaming-first-chunk.json"), "lazy")]
#[case::pre_stream_http_error(include_str!("tests/fixtures/pre-stream-http-error.json"), "error")]
#[case::stream_exception_frame(include_str!("tests/fixtures/stream-exception-frame.json"), "eof")]
#[case::sync_async_beta_inputs(include_str!("tests/fixtures/sync-async-beta-inputs.json"), "beta")]
fn converse_call_cases(#[case] source: &str, #[case] scenario: &str) {
    let input: fixture::Input = serde_json::from_str(source).unwrap();
    let output: fixture::Observation = fixture::run(&input).unwrap();
    assert!(!output.url.is_empty(), "{output:?}");
    assert_eq!(output.sends, 1);
    match scenario {
        "signed" => signed_output(&output),
        "bearer" => bearer_output(&output),
        "buffered" => buffered_output(&output),
        "fake" => {
            assert!(output.url.ends_with("/converse"));
            assert!(output.stream);
            assert_eq!(output.chunks, 1);
        }
        "lazy" => {
            assert!(output.stream);
            assert_eq!(output.reads, 0);
            assert_eq!(output.chunks, 0);
        }
        "error" => {
            assert_eq!(output.status, Some(429));
            assert_eq!(output.content, None);
        }
        "eof" => {
            assert!(output.stream);
            assert_eq!(output.status, Some(502));
            assert!(output.error.as_deref().unwrap().contains("no events"));
        }
        "beta" => {
            assert!(!output.headers.contains_key("anthropic-beta"));
            assert_eq!(output.content.as_deref(), Some("hello"));
        }
        _ => panic!("unknown fixture scenario"),
    }
}

fn signed_output(output: &fixture::Observation) {
    assert!(output.url.ends_with("/model/model/converse"));
    assert_eq!(output.credentials, 1);
    assert!(
        output
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("authorization")
                && value.contains("/us-east-1/bedrock/aws4_request"))
    );
}

fn bearer_output(output: &fixture::Observation) {
    assert_eq!(output.credentials, 0);
    assert_eq!(
        output.headers.get("Authorization").map(String::as_str),
        Some("Bearer fixture-bearer")
    );
    assert_eq!(
        output.headers.get("Content-Type").map(String::as_str),
        Some("application/custom")
    );
}

fn buffered_output(output: &fixture::Observation) {
    assert_eq!(output.content.as_deref(), Some("hello"));
    assert!(!output.stream);
    assert_eq!(output.status, None);
    assert!(output.body.contains("hello"));
    assert_eq!(output.signed_body, output.body.as_bytes());
}

#[test]
fn bedrock_converse_call_credential_identity_isolation() {
    let mut first: fixture::Input =
        serde_json::from_str(include_str!("tests/fixtures/buffered-response.json")).unwrap();
    first.bearer = None;
    first.params = serde_json::from_str(r#"{"aws_access_key_id":"identity-one","aws_secret_access_key":"secret-one","aws_session_token":"token-one"}"#).unwrap();
    let one = fixture::run(&first).unwrap();
    let mut second = first.clone();
    second.params = serde_json::from_str(r#"{"aws_access_key_id":"identity-two","aws_secret_access_key":"secret-two","aws_session_token":"token-two"}"#).unwrap();
    let two = fixture::run(&second).unwrap();
    let again = fixture::run(&first).unwrap();
    assert_eq!(
        (one.credentials, two.credentials, again.credentials),
        (1, 1, 1)
    );
    assert_eq!(one.headers, again.headers);
    assert_ne!(one.headers, two.headers);
    assert!(
        one.headers
            .values()
            .any(|value| value.contains("Credential=identity-one/"))
    );
    assert!(
        two.headers
            .values()
            .any(|value| value.contains("Credential=identity-two/"))
    );
    assert!(!format!("{:?}", two.headers).contains("identity-one"));
}

#[rstest::rstest]
#[case::default_sync(false, None, 1)]
#[case::default_async(true, None, 1)]
#[case::supplied_sync(false, Some(ClientKind::Sync), 0)]
#[case::supplied_async(true, Some(ClientKind::Asynchronous), 0)]
#[case::discard_sync(true, Some(ClientKind::Sync), 1)]
#[case::discard_async(false, Some(ClientKind::Asynchronous), 1)]
fn client_selection(
    #[case] asynchronous: bool,
    #[case] supplied: Option<ClientKind>,
    #[case] factories: usize,
) {
    let mut input: fixture::Input =
        serde_json::from_str(include_str!("tests/fixtures/buffered-response.json")).unwrap();
    input.asynchronous = asynchronous;
    input.native.supplied = supplied;
    let reply = fixture::run(&input).unwrap();
    assert_eq!(reply.factory, factories);
    assert_eq!(reply.content.as_deref(), Some("hello"));
    input
        .params
        .insert("stream".to_owned(), ParamValue::Bool(true));
    let stream = fixture::run(&input).unwrap();
    assert_eq!(stream.factory, factories);
    assert!(stream.stream);
    assert_eq!(
        stream.factory_timeout_ms,
        if factories == 0 || asynchronous {
            None
        } else {
            Some(3000)
        }
    );
}

#[rstest::rstest]
#[case::buffered_timeout(false, fixture::input::Failure::Timeout, Some(408))]
#[case::buffered_closed(false, fixture::input::Failure::Closed, None)]
#[case::stream_timeout(true, fixture::input::Failure::Timeout, Some(408))]
#[case::stream_closed(true, fixture::input::Failure::Closed, Some(500))]
fn async_transport_failure(
    #[case] streaming: bool,
    #[case] failure: fixture::input::Failure,
    #[case] status: Option<u16>,
) {
    let mut input: fixture::Input =
        serde_json::from_str(include_str!("tests/fixtures/buffered-response.json")).unwrap();
    input.native.failure = failure;
    input
        .params
        .insert("stream".to_owned(), ParamValue::Bool(streaming));
    let output = fixture::run(&input).unwrap();
    assert_eq!(output.status, status);
    assert!(output.error.is_some());
    assert!(!output.stream);
}

#[rstest::rstest]
#[case::sync((false, false), 2)]
#[case::async_logged((true, false), 2)]
#[case::async_skipped((true, true), 1)]
fn pre_call_logging_suppression(#[case] routing: (bool, bool), #[case] logs: usize) {
    let mut input: fixture::Input =
        serde_json::from_str(include_str!("tests/fixtures/buffered-response.json")).unwrap();
    input.asynchronous = routing.0;
    input.native.skip_log = routing.1;
    let output = fixture::run(&input).unwrap();
    assert_eq!(output.logs.len(), logs);
    assert_eq!(output.content.as_deref(), Some("hello"));
    input
        .params
        .insert("stream".to_owned(), ParamValue::Bool(true));
    let stream = fixture::run(&input).unwrap();
    assert_eq!(stream.logs.len(), 2);
    assert_eq!(stream.reads, 0);
}

#[rstest::rstest]
#[case::throttle(include_str!("tests/fixtures/throttlingException-before.json"), 429, 0)]
#[case::throttle_after(include_str!("tests/fixtures/throttlingException-after.json"), 429, 1)]
#[case::model(include_str!("tests/fixtures/modelStreamErrorException-before.json"), 424, 0)]
#[case::model_after(include_str!("tests/fixtures/modelStreamErrorException-after.json"), 424, 1)]
#[case::validation(include_str!("tests/fixtures/validationException-before.json"), 400, 0)]
#[case::internal(include_str!("tests/fixtures/internalServerException-after.json"), 503, 1)]
#[case::unavailable(include_str!("tests/fixtures/serviceUnavailableException-before.json"), 503, 0)]
#[case::timeout(include_str!("tests/fixtures/modelTimeoutException-after.json"), 408, 1)]
fn modeled_stream_errors_cancel_body(
    #[case] json: &str,
    #[case] status: u16,
    #[case] chunks: usize,
) {
    let input: fixture::Input = serde_json::from_str(json).unwrap();
    let output = fixture::run(&input).unwrap();
    assert!(output.stream);
    assert_eq!(output.status, Some(status));
    assert_eq!(output.chunks, chunks);
    assert_eq!(output.reads, chunks.checked_add(1).unwrap());
    assert_eq!(output.drops, 1);
    assert!(
        output
            .error
            .as_deref()
            .unwrap()
            .contains(&status.to_string())
    );
}

#[rstest::rstest]
#[case::sync_before(include_str!("tests/fixtures/internalServerException-before.json"), false)]
#[case::sync_after(include_str!("tests/fixtures/internalServerException-after.json"), false)]
#[case::async_before(include_str!("tests/fixtures/internalServerException-before.json"), true)]
#[case::async_after(include_str!("tests/fixtures/internalServerException-after.json"), true)]
fn internal_stream_error_uses_shared_class(#[case] json: &str, #[case] asynchronous: bool) {
    let mut input: fixture::Input = serde_json::from_str(json).unwrap();
    input.asynchronous = asynchronous;
    let output = fixture::run(&input).unwrap();
    let class = output.mapped_error.unwrap();
    assert!(matches!(class, ErrorClass::ServiceUnavailable(_)));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 503);
    assert_eq!(metadata.model.as_deref(), Some(input.model.as_str()));
    assert_eq!(metadata.provider.as_deref(), Some("bedrock"));
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
    assert_eq!(output.reads, input.frames.len().checked_sub(1).unwrap());
    assert_eq!(output.chunks, output.reads.checked_sub(1).unwrap());
    assert_eq!(output.drops, 1);
}

#[test]
fn malformed_stream_frame_uses_shared_connection_error() {
    let mut input: fixture::Input =
        serde_json::from_str(include_str!("tests/fixtures/streaming-first-chunk.json")).unwrap();
    input.frames = vec![vec![0; 12]];
    input.consume = true;
    let output = fixture::run(&input).unwrap();
    assert!(matches!(
        output.mapped_error,
        Some(ErrorClass::ApiConnection(_))
    ));
    assert_eq!(output.status, Some(500));
    assert!(output.error.as_deref().unwrap().contains("checksum"));
    assert_eq!(output.chunks, 0);
    assert_eq!(output.drops, 1);
}

#[test]
fn unconsumed_stream_drops_without_reading() {
    let mut input: fixture::Input =
        serde_json::from_str(include_str!("tests/fixtures/streaming-first-chunk.json")).unwrap();
    input.native.chunk_size = Some(19);
    let output = fixture::run(&input).unwrap();
    assert_eq!(output.wrapper_model.as_deref(), Some(input.model.as_str()));
    assert_eq!(output.provider.as_deref(), Some("bedrock"));
    assert_eq!(
        output
            .response_headers
            .get("x-request-id")
            .map(String::as_str),
        Some("request-1")
    );
    assert_eq!(output.chunk_size, Some(19));
    assert_eq!(output.reads, 0);
    assert_eq!(output.drops, 1);
}

#[test]
fn buffered_uses_injected_tokenizer_and_keeps_response_headers() {
    let mut input: fixture::Input =
        serde_json::from_str(include_str!("tests/fixtures/buffered-response.json")).unwrap();
    r#"{"output":{"message":{"role":"assistant","content":[{"text":"hello"},{"reasoningContent":{"reasoningText":{"text":"thinking","signature":"sig"}}}]}},"stopReason":"end_turn","usage":{"inputTokens":10,"outputTokens":5,"totalTokens":15}}"#.clone_into(&mut input.body);
    let output = fixture::run(&input).unwrap();
    assert_eq!(output.reasoning_tokens, Some(2));
    assert_eq!(
        output
            .response_headers
            .get("x-request-id")
            .map(String::as_str),
        Some("request-1")
    );
    assert_eq!(output.body.as_bytes(), output.signed_body);
}

#[test]
fn first_chunk_is_delivered_on_demand() {
    let mut input: fixture::Input =
        serde_json::from_str(include_str!("tests/fixtures/streaming-first-chunk.json")).unwrap();
    input.consume = true;
    let output = fixture::run(&input).unwrap();
    assert_eq!(output.chunks, 1);
    assert_eq!(output.reads, 1);
    assert_eq!(output.status, None);
}

#[rstest::rstest]
#[case::sync(false)]
#[case::asynchronous(true)]
fn client_beta_body_uses_route_specific_headers(#[case] asynchronous: bool) {
    let mut input: fixture::Input =
        serde_json::from_str(include_str!("tests/fixtures/sync-async-beta-inputs.json")).unwrap();
    input.asynchronous = asynchronous;
    let output = fixture::run(&input).unwrap();
    assert_eq!(output.body.contains("unknown-beta"), !asynchronous);
    assert!(!output.headers.contains_key("anthropic-beta"));
    assert_eq!(output.signed_body, output.body.as_bytes());
}

#[rstest::rstest]
#[case::sync(false, None)]
#[case::asynchronous(true, Some(500))]
fn fake_stream_conversion_failure_uses_route_error_policy(
    #[case] asynchronous: bool,
    #[case] status: Option<u16>,
) {
    let mut input: fixture::Input =
        serde_json::from_str(include_str!("tests/fixtures/converse-stream-path.json")).unwrap();
    input.asynchronous = asynchronous;
    r#"{"output":{"message":{"role":"assistant","content":[{"text":"hello"}]}},"stopReason":"end_turn"}"#.clone_into(&mut input.body);
    let observed = fixture::run(&input).unwrap();
    assert_eq!(observed.status, status, "{observed:?}");
    assert!(observed.error.is_some());
    assert!(!observed.stream);
    assert_eq!(observed.sends, 1);
    assert_eq!(observed.reads, 1);
    assert_eq!(observed.drops, 1);
}
