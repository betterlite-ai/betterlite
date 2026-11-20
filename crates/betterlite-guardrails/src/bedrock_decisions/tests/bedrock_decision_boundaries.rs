use std::collections::BTreeMap;

use rstest::rstest;

use super::{
    DiscardSink, as_json, base, call_log, catalog, context, empty_request_data, json, response,
};
use crate::bedrock_decisions::error::BedrockDecisionError;
use crate::bedrock_decisions::exceptions::{
    BlockedGuardrail, blocked_guardrail_failure, body_has_exception, merged_response_status,
    payload_has_exception, response_has_exception, response_status,
};
use crate::bedrock_decisions::logging::log_failure;
use crate::bedrock_records::Usage;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_records::record::GuardrailStatus;

#[rstest]
#[case(r#"{"Output":{"__type":"ValidationException"}}"#, true)]
#[case(r#"{"Output":{"__type":"Exception"}}"#, true)]
#[case(
    r#"{"Output":{"__type":"InternalServerException"},"action":"NONE"}"#,
    true
)]
#[case(r#"{"Output":{"__type":"NonexceptionMarker"}}"#, false)]
#[case(r#"{"Output":{"__type":"nonexception"}}"#, false)]
#[case(r#"{"Output":{"__type":""}}"#, false)]
#[case(r#"{"Output":{"__type":null}}"#, false)]
#[case(r#"{"Output":{"__type":5}}"#, false)]
#[case(r#"{"Output":{"__type":["Exception"]}}"#, false)]
#[case(r#"{"Output":null}"#, false)]
#[case(r#"{"Output":"ValidationException"}"#, false)]
#[case(r#"{"Output":["ValidationException"]}"#, false)]
#[case(r#"{"output":{"__type":"ValidationException"}}"#, false)]
#[case(r#"{"__type":"ValidationException"}"#, false)]
#[case(r#"{"Exception":"ValidationException"}"#, false)]
#[case("[]", false)]
#[case(r#""ValidationException""#, false)]
#[case("null", false)]
#[case("1.5", false)]
#[case(r#"{"Output":{"__type":"Exception"},"x":1.0}"#, true)]
fn payload_has_exception_needs_a_text_marker_under_output(
    #[case] wire: &str,
    #[case] expected: bool,
) {
    assert_eq!(payload_has_exception(&json(wire)), expected);
}

#[test]
fn body_has_exception_decodes_the_body_as_utf8_json() {
    assert!(body_has_exception(
        br#"{"Output":{"__type":"ValidationException"}}"#
    ));
    assert!(body_has_exception(
        "{\"Output\":{\"__type\":\"ValidationException\"}}\n".as_bytes()
    ));
    assert!(!body_has_exception(b"not json"));
    assert!(!body_has_exception(b""));
    assert!(!body_has_exception(b"\xff\xfe"));
    assert!(!body_has_exception(
        b"\xef\xbb\xbf{\"Output\":{\"__type\":\"Exception\"}}"
    ));
}

#[test]
fn body_has_exception_returns_false_for_nesting_past_the_decoder_limit() {
    let deep = format!("{}1{}", "[".repeat(300), "]".repeat(300));
    assert!(!body_has_exception(deep.as_bytes()));
}

#[test]
fn response_has_exception_reads_the_output_kept_in_extra() {
    assert!(response_has_exception(&response(
        r#"{"Output":{"__type":"ValidationException"}}"#
    )));
    assert!(!response_has_exception(&response(r#"{"Output":"text"}"#)));
    assert!(!response_has_exception(&response(r#"{"Output":null}"#)));
    assert!(!response_has_exception(&response(r#"{"action":"NONE"}"#)));
}

#[test]
fn merged_response_status_keeps_a_marker_carried_along_the_merge() {
    let mut merged = response(r#"{"action":"NONE","outputs":[{"text":"first"}]}"#);
    merged.absorb(&response(
        r#"{"Output":{"__type":"ValidationException"},"outputs":[{"text":"second"}]}"#,
    ));
    assert_eq!(
        merged_response_status(&merged),
        GuardrailStatus::GuardrailFailedToRespond
    );
    let mut marked = response(r#"{"Output":{"__type":"ValidationException"}}"#);
    marked.absorb(&response(r#"{"action":"NONE"}"#));
    assert_eq!(
        merged_response_status(&marked),
        GuardrailStatus::GuardrailFailedToRespond
    );
    let mut clean = response(r#"{"action":"NONE"}"#);
    clean.absorb(&response(r#"{"outputs":[{"text":"second"}]}"#));
    assert_eq!(merged_response_status(&clean), GuardrailStatus::Success);
}

#[rstest]
#[case(0)]
#[case(199)]
#[case(204)]
#[case(302)]
#[case(599)]
fn response_status_treats_every_non_200_as_failed(#[case] status: u16) {
    assert_eq!(
        response_status(status, br#"{"action":"NONE"}"#),
        GuardrailStatus::GuardrailFailedToRespond
    );
}

#[test]
fn response_status_falls_back_to_success_when_the_body_cannot_be_decoded() {
    assert_eq!(response_status(200, b"\xff\xfe"), GuardrailStatus::Success);
    assert_eq!(
        response_status(200, br#"{"assessments":"x"}"#),
        GuardrailStatus::Success
    );
    let deep = format!("{{\"a\":{}{}}}", "[".repeat(300), "]".repeat(300));
    assert_eq!(
        response_status(200, deep.as_bytes()),
        GuardrailStatus::Success
    );
}

#[test]
fn http_exception_omits_assessments_without_a_blocked_action() {
    let merged = response(
        r#"{"outputs":[{"text":"masked"}],"assessments":[{"sensitiveInformationPolicy":{"piiEntities":[{"type":"EMAIL","match":"a@b.c","action":"ANONYMIZED"}]}}]}"#,
    );
    let guardrail = BlockedGuardrail {
        identifier: None,
        version: None,
        name: None,
        disable_exception_on_block: false,
    };
    let failure = blocked_guardrail_failure(&merged, &empty_request_data(), &guardrail);
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("a blocked guardrail without disable_exception_on_block is an HTTP object failure");
    };
    assert_eq!(status, 400);
    let expected =
        json(r#"{"error":"Violated guardrail policy","bedrock_guardrail_response":"masked"}"#);
    assert_eq!(as_json(&detail), expected);
}

#[test]
fn log_failure_reports_a_counter_that_overflows_i64() {
    let prices = catalog(&[("bedrock/guardrails", r#"{"contentPolicyUnits":0.5}"#)]);
    let call = call_log(&prices);
    let guardrail_base = base(Some("gr-name"));
    let mut request_data = empty_request_data();
    let sink = DiscardSink;
    let mut record_context = context(&guardrail_base, &mut request_data, &sink);
    let completed = [
        Usage(BTreeMap::from([(
            "contentPolicyUnits".to_owned(),
            Some(i64::MAX),
        )])),
        Usage(BTreeMap::from([("contentPolicyUnits".to_owned(), Some(1))])),
    ];
    let result = log_failure(&mut record_context, &call, "boom", &completed);
    assert!(matches!(
        result,
        Err(BedrockDecisionError::UsageOverflow { counter }) if counter == "contentPolicyUnits"
    ));
    assert!(request_data.is_empty());
}
