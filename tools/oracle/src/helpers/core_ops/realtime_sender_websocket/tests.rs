#![cfg(test)]

use std::error::Error as _;
use std::iter::{once, repeat_n};
use std::sync::{Arc, Mutex};

use betterlite_core::api::realtime_contracts::config::BaseRealtimeConfig as _;
use betterlite_core::api::realtime_contracts::error::WebSocketError;
use betterlite_oracle_runner::case::helper::HelperCase;
use rstest::rstest;
use serde_json::{Value, json};

use super::driver::drive;
use super::input::{
    Input, MAX_CAPTURE_BYTES, MAX_OPERATIONS, MAX_TEXT_BYTES, Operation, ProviderDocument,
};
use super::provider::{ScriptError, ScriptedProvider};
use super::transport::capture;

const EMPTY_INPUT: &str = r#"{
    "model":"provider-model", "force_transcription_model":null,
    "client_wants_beta":false, "backend_uses_beta_protocol":null, "provider":null,
    "guardrails":[], "backend_failures":[], "client_failures":[], "operations":[]
}"#;

// MAX_TEXT_BYTES is even and a two-byte character fills it with exactly this many characters
const TWO_BYTE_CHARACTERS: usize = MAX_TEXT_BYTES >> 1;

#[rstest]
#[case::inside(MAX_OPERATIONS - 1, true)]
#[case::limit(MAX_OPERATIONS, true)]
#[case::outside(MAX_OPERATIONS + 1, false)]
fn disable_operations_are_bounded(#[case] count: usize, #[case] accepted: bool) {
    let mut input: Input = serde_json::from_str(EMPTY_INPUT).unwrap();
    input.operations = repeat_n(Operation::Disable, count).collect();
    assert_eq!(input.check().is_ok(), accepted);
}

#[rstest]
#[case::inside(MAX_TEXT_BYTES - 1, true)]
#[case::limit(MAX_TEXT_BYTES, true)]
#[case::outside(MAX_TEXT_BYTES + 1, false)]
fn model_bytes_are_bounded(#[case] bytes: usize, #[case] accepted: bool) {
    let mut input: Input = serde_json::from_str(EMPTY_INPUT).unwrap();
    input.force_transcription_model = Some("a".repeat(bytes));
    assert_eq!(input.check().is_ok(), accepted);
    input.model = input.force_transcription_model.take().unwrap();
    assert_eq!(input.check().is_ok(), accepted);
}

#[rstest]
#[case::inside(TWO_BYTE_CHARACTERS - 1, true)]
#[case::limit(TWO_BYTE_CHARACTERS, true)]
#[case::outside(TWO_BYTE_CHARACTERS + 1, false)]
fn operation_limits_count_utf8_bytes(#[case] characters: usize, #[case] accepted: bool) {
    let mut input: Input = serde_json::from_str(EMPTY_INPUT).unwrap();
    input.operations.push(Operation::Send {
        text: "\u{e9}".repeat(characters),
    });
    assert_eq!(input.check().is_ok(), accepted);
}

#[rstest]
#[case::frame_inside(0, MAX_TEXT_BYTES - 1, true)]
#[case::frame_limit(0, MAX_TEXT_BYTES, true)]
#[case::frame_outside(0, MAX_TEXT_BYTES + 1, false)]
#[case::capture_inside(MAX_CAPTURE_BYTES - 2, 1, true)]
#[case::capture_limit(MAX_CAPTURE_BYTES - 1, 1, true)]
#[case::capture_outside(MAX_CAPTURE_BYTES, 1, false)]
fn generated_capture_limits(#[case] used: usize, #[case] size: usize, #[case] accepted: bool) {
    let budget = Arc::new(Mutex::new(used));
    assert_eq!(capture(&budget, &[size]).is_ok(), accepted);
    let expected = if accepted {
        used.saturating_add(size)
    } else {
        used
    };
    assert_eq!(*budget.lock().unwrap(), expected);
}

#[test]
fn enforced_frames_are_bounded_before_send() {
    let mut input: Input = serde_json::from_str(EMPTY_INPUT).unwrap();
    input.force_transcription_model = Some("m".repeat(MAX_TEXT_BYTES));
    input.operations.push(Operation::Send {
        text: String::from(
            r#"{"type":"session.update","session":{"input_audio_transcription":{}}}"#,
        ),
    });
    input.check().unwrap();
    let actual = drive(&input).unwrap();
    assert_eq!(actual["results"], json!([{"error":"backend"}]));
    assert_eq!(actual["upstream"], json!([]));
}

fn cases(source: &str) -> Vec<Input> {
    let case: HelperCase = serde_saphyr::from_str(source).unwrap();
    once(case.call)
        .chain(case.calls)
        .map(|call| serde_json::from_value(call.input).unwrap())
        .collect()
}

#[test]
fn provider_receives_enforced_message_and_cached_setup() {
    let inputs = cases(include_str!(
        "../../../../cases/websocket/realtime-sender/realtime-authorized-transcription-model.yaml"
    ));
    let input = inputs
        .iter()
        .find(|input| input.model == "provider-model" && input.provider.is_some())
        .unwrap();
    let actual = drive(input).unwrap();
    assert_eq!(actual["results"], json!([{"ok":true},{"ok":false}]));
    assert_eq!(
        actual["timeline"],
        json!([{"transform": {
            "message": {"type":"session.update", "session": {
                "input_audio_transcription":{"model":"allowed"},
                "audio":{"input":{"transcription":{"model":"allowed"}}}
            }},
            "model":"provider-model",
            "session_configuration_request": r#"{"setup":{"model":"cached-model"}}"#
        }}])
    );
}

#[test]
fn binary_failure_fixture_reaches_backend_send() {
    let inputs = cases(include_str!(
        "../../../../cases/websocket/realtime-sender/realtime-backend-send-order.yaml"
    ));
    let actual = drive(&inputs[3]).unwrap();
    assert_eq!(
        actual["results"],
        json!([
            {"error":"backend"},{"error":"backend"},{"ok":true},{"ok":false}
        ])
    );
    let upstream: Vec<Value> = serde_json::from_value(actual["upstream"].clone()).unwrap();
    assert_eq!(upstream.len(), 1);
    assert_eq!(upstream[0]["wire"], "binary");
    assert_eq!(actual["state"]["content_sent_after_setup"], true);
}

#[test]
fn aggregate_transform_capture_is_bounded() {
    let inputs = cases(include_str!(
        "../../../../cases/websocket/realtime-sender/realtime-authorized-transcription-model.yaml"
    ));
    let actual = drive(inputs.last().unwrap()).unwrap();
    assert_eq!(actual["results"][21], json!({"ok":false}));
    assert_eq!(actual["results"][22], json!({"error":"transform"}));
    assert_eq!(actual["timeline"].as_array().unwrap().len(), 21);
    assert_eq!(actual["upstream"], json!([]));
}

#[rstest]
#[case::injected(0, None)]
#[case::capture(MAX_CAPTURE_BYTES, Some("the scripted capture is past its byte limit"))]
fn provider_failure_preserves_its_source(
    #[case] used: usize,
    #[case] expected_source: Option<&str>,
) {
    let provider = ScriptedProvider::new(
        &ProviderDocument {
            requires_session_configuration: false,
            setup: Vec::new(),
            content: Vec::new(),
            transforms: Vec::new(),
            fail_transform: true,
        },
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(used)),
    );
    let error = provider
        .transform_realtime_request("{}", "model", None)
        .unwrap_err();
    assert_eq!(
        matches!(&error, ScriptError::Capture { .. }),
        expected_source.is_some()
    );
    assert_eq!(
        error.source().map(|source| source
            .downcast_ref::<WebSocketError>()
            .unwrap()
            .message
            .as_str()),
        expected_source
    );
}
