#![cfg(test)]

use super::super::Op;
use super::normalize_warmup;
use crate::websocket_driver::service::{ServiceInput, ServiceTag};
use crate::websocket_driver::{TranscriptObservation, compare_transcript};
use crate::websocket_oracle::OracleLimits;
use crate::websocket_oracle::decode::{FilesystemFixtures, decode_ws_case};
use betterlite_oracle_helper_api::helper::Helper as _;
use rstest::rstest;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

#[rstest]
#[case::invalid_and_ignored("invalid-and-ignored")]
#[case::warmup("warmup")]
#[case::warmup_model_fallback("warmup-model-fallback")]
#[case::warmup_send_failure("warmup-send-failure")]
#[case::rate_limit("rate-limit")]
#[case::routing_and_metadata("routing-and-metadata")]
#[case::different_provider_model("different-provider-model")]
#[case::stream_forwarding("stream-forwarding")]
#[case::stream_send_failure("stream-send-failure")]
#[case::two_turn_history("two-turn-history")]
#[case::history_fallback_and_failure("history-fallback-and-failure")]
#[case::history_guards("history-guards")]
#[case::run_lifecycle("run-lifecycle")]
fn bounded_managed_transcripts(#[case] name: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bytes = fs::read(
        root.join("cases/websocket/responses-ws-managed")
            .join(format!("{name}.yaml")),
    )
    .unwrap();
    let case = decode_ws_case(
        &bytes,
        &OracleLimits::default(),
        &FilesystemFixtures {
            root: root.join("fixtures/responses-ws-managed"),
        },
    )
    .unwrap();
    let input = ServiceInput {
        service: ServiceTag::ManagedResponsesTurns,
        input: case.input.clone(),
    };
    let wire = serde_json::to_value(input.to_wire()).unwrap();
    let restored: ServiceInput = serde_json::from_value(wire).unwrap();
    let answer = Op::run(&restored).unwrap();
    let observed: TranscriptObservation = serde_json::from_value(answer.clone()).unwrap();
    compare_transcript(&case, &observed).unwrap();
    assert_eq!(
        answer["managed"].as_array().unwrap().len(),
        case.input.managed.len()
    );
    if name == "routing-and-metadata" {
        assert_eq!(
            answer.pointer("/managed/0/provider_requests/0/body/stream"),
            Some(&json!(false))
        );
    }
}

#[rstest]
#[case::valid("resp_warmup_0123456789abcdef0123456789abcdef", 1, true)]
#[case::short("resp_warmup_abc", 1, false)]
#[case::uppercase("resp_warmup_0123456789ABCDEF0123456789ABCDEF", 1, false)]
#[case::negative_clock("resp_warmup_0123456789abcdef0123456789abcdef", -1, false)]
fn warmup_normalization_validates_first(
    #[case] id: &str,
    #[case] timestamp: i64,
    #[case] accepted: bool,
) {
    let mut frame = json!({"response":{"id":id,"created_at":timestamp,"extra":{"present":null}}});
    let mut previous = None;
    assert_eq!(
        normalize_warmup(&mut frame, 0, &mut previous).is_ok(),
        accepted
    );
    if accepted {
        assert_eq!(frame.pointer("/response/created_at"), Some(&json!(0)));
        assert_eq!(frame.pointer("/response/extra/present"), Some(&json!(null)));
        let mut changed = json!({"response":{"id":"resp_warmup_abcdef0123456789abcdef0123456789","created_at":1}});
        normalize_warmup(&mut changed, 1, &mut previous).unwrap_err();
    }
}
