#![cfg(test)]

use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::straiker::delivery::fixtures::{
    Fakes, V3_ALLOW, V3_BLOCK, capture_warnings, delivery, inputs, raw, reply, request_data, run,
    settings, v3_raw,
};
use crate::straiker::delivery::responses::{Verdict, VerdictAction, parse, parse_v1, parse_v3};
use crate::straiker::delivery::settings::ApiVersion;

fn verdict_v1(body: &str) -> Verdict {
    parse_v1(body).expect("a valid v1 answer")
}

fn verdict_v3(body: &str) -> Verdict {
    parse_v3(body).expect("a valid v3 answer")
}

#[test]
fn a_v1_block_is_read_with_its_reason_and_controls() {
    let verdict = verdict_v1(
        r#"{"action":"BLOCKED","blocked_reason":"policy","blocked_by":["pii"],"turnId":"turn-1"}"#,
    );
    assert_eq!(verdict.action, VerdictAction::Blocked);
    assert_eq!(verdict.blocked_reason.as_deref(), Some("policy"));
    assert_eq!(verdict.blocked_by, vec!["pii".to_owned()]);
    assert_eq!(verdict.turn_id.as_deref(), Some("turn-1"));
}

#[test]
fn an_empty_v1_answer_is_no_action() {
    let verdict = verdict_v1("{}");
    assert_eq!(verdict.action, VerdictAction::None);
    assert!(verdict.blocked_reason.is_none());
    assert!(verdict.blocked_by.is_empty());
    assert!(verdict.texts.is_none());
    assert!(verdict.schema_version.is_none());
    assert!(verdict.turn_id.is_none());
}

#[test]
fn a_v1_intervention_carries_the_replacement_texts() {
    let verdict = verdict_v1(r#"{"action":"GUARDRAIL_INTERVENED","texts":["rewritten"]}"#);
    assert_eq!(verdict.action, VerdictAction::Intervened);
    assert_eq!(verdict.texts, Some(vec!["rewritten".to_owned()]));
}

#[test]
fn a_v1_verdict_carries_its_schema_version() {
    let verdict = verdict_v1(r#"{"action":"NONE","schema_version":"2"}"#);
    assert_eq!(verdict.schema_version.as_deref(), Some("2"));
}

#[test]
fn a_v1_verdict_reads_only_the_camel_case_turn_id() {
    assert_eq!(
        verdict_v1(r#"{"turnId":"camel","turn_id":"snake"}"#)
            .turn_id
            .as_deref(),
        Some("camel")
    );
    assert!(verdict_v1(r#"{"turn_id":"snake"}"#).turn_id.is_none());
}

#[rstest]
#[case::list("[]", "list")]
#[case::null("null", "NoneType")]
#[case::text(r#""x""#, "str")]
#[case::integer("1", "int")]
#[case::float("1.5", "float")]
#[case::boolean("true", "bool")]
fn a_v1_answer_that_is_not_an_object_names_its_type(#[case] body: &str, #[case] kind: &str) {
    let error = parse_v1(body).expect_err("the answer is not an object");
    assert_eq!(
        error.message,
        format!("invalid response schema: expected an object, got {kind}")
    );
}

#[test]
fn a_v1_answer_that_is_not_json_is_a_schema_error() {
    let error = parse_v1("{").expect_err("the answer is not JSON");
    assert!(error.message.starts_with("invalid response schema: "));
    assert!(!error.retryable);
    assert!(!error.is_unreachable);
}

#[test]
fn a_v1_answer_with_an_unknown_action_is_a_schema_error() {
    let error = parse_v1(r#"{"action":"MAYBE"}"#).expect_err("the action is unknown");
    assert!(error.message.starts_with("invalid response schema: "));
}

#[test]
fn the_api_version_picks_the_reader() {
    let flat_block = r#"{"action":"block","blocked_by":["pii"]}"#;
    assert_eq!(
        parse(ApiVersion::V3, flat_block)
            .expect("a v3 answer")
            .action,
        VerdictAction::Blocked
    );
    assert!(parse(ApiVersion::V1, flat_block).is_err());
    assert_eq!(
        parse(ApiVersion::V1, r#"{"action":"BLOCKED"}"#)
            .expect("a v1 answer")
            .action,
        VerdictAction::Blocked
    );
    assert_eq!(
        parse(ApiVersion::V3, r#"{"action":"BLOCKED"}"#)
            .expect("a v3 answer")
            .action,
        VerdictAction::None
    );
}

#[test]
fn a_v3_block_is_read_with_its_message_and_controls() {
    let verdict = verdict_v3(
        r#"{"straiker":{"action":"block","blocked_by":["pii"],"block_message":"policy","turn_id":"turn-1"}}"#,
    );
    assert_eq!(verdict.action, VerdictAction::Blocked);
    assert_eq!(verdict.blocked_reason.as_deref(), Some("policy"));
    assert_eq!(verdict.blocked_by, vec!["pii".to_owned()]);
    assert_eq!(verdict.turn_id.as_deref(), Some("turn-1"));
}

#[test]
fn a_v3_allow_is_no_action() {
    let verdict = verdict_v3(r#"{"straiker":{"action":"allow"}}"#);
    assert_eq!(verdict.action, VerdictAction::None);
    assert!(verdict.blocked_reason.is_none());
    assert!(verdict.blocked_by.is_empty());
}

#[test]
fn a_v3_block_without_controls_names_the_policy() {
    let verdict = verdict_v3(r#"{"straiker":{"action":"block"}}"#);
    assert_eq!(verdict.action, VerdictAction::Blocked);
    assert_eq!(
        verdict.blocked_reason.as_deref(),
        Some("Straiker blocked this turn: policy")
    );
}

#[test]
fn a_hook_decision_blocks_without_a_flat_action() {
    let verdict = verdict_v3(r#"{"hookSpecificOutput":{"permissionDecision":"deny"}}"#);
    assert_eq!(verdict.action, VerdictAction::Blocked);
    assert_eq!(
        verdict.blocked_reason.as_deref(),
        Some("Straiker blocked this turn: policy")
    );
}

#[test]
fn a_blocking_control_blocks_even_beside_an_allow() {
    let verdict = verdict_v3(r#"{"straiker":{"action":"allow","blocked_by":["pii"]}}"#);
    assert_eq!(verdict.action, VerdictAction::Blocked);
    assert_eq!(
        verdict.blocked_reason.as_deref(),
        Some("Straiker blocked this turn: pii")
    );
    assert_eq!(verdict.blocked_by, vec!["pii".to_owned()]);
}

#[rstest]
#[case::empty("{}")]
#[case::asked(r#"{"straiker":{"action":"ask"}}"#)]
fn a_v3_answer_without_a_decision_is_a_schema_error(#[case] body: &str) {
    let error = parse_v3(body).expect_err("there is no verdict");
    assert_eq!(
        error.message,
        "invalid response schema: no allow or block decision"
    );
}

#[test]
fn a_flat_blocked_word_is_not_a_block() {
    let verdict = verdict_v3(r#"{"action":"blocked"}"#);
    assert_eq!(verdict.action, VerdictAction::None);
}

#[test]
fn a_v3_turn_id_falls_back_to_the_body() {
    let verdict =
        verdict_v3(r#"{"straiker":{"action":"block","blocked_by":["x"]},"turn_id":"turn-9"}"#);
    assert_eq!(verdict.turn_id.as_deref(), Some("turn-9"));
}

#[test]
fn a_v3_answer_that_is_not_an_object_names_its_type() {
    let error = parse_v3("[]").expect_err("the answer is not an object");
    assert_eq!(
        error.message,
        "invalid response schema: expected an object, got list"
    );
}

#[tokio::test]
async fn a_v1_schema_drift_is_logged() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&raw()),
        vec![reply(200, r#"{"action":"NONE","schema_version":"2"}"#)],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let (warnings, _guard) = capture_warnings();
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert_eq!(warnings.take(), vec!["straiker.schema_drift"]);
}

#[tokio::test]
async fn the_current_schema_version_is_not_a_drift() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&raw()),
        vec![reply(200, r#"{"action":"NONE","schema_version":"1"}"#)],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let (warnings, _guard) = capture_warnings();
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert!(warnings.take().is_empty());
}

#[tokio::test]
async fn a_v3_reply_is_read_by_the_v3_reader() {
    let fakes = Fakes::default();
    let blocked_delivery = delivery(settings(&v3_raw()), vec![reply(200, V3_BLOCK)]);
    let mut data =
        request_data(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &blocked_delivery,
        &mut data,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the answer blocks");
    let GuardrailFailure::GuardrailRaised { message, .. } = failure else {
        panic!("expected a block");
    };
    assert_eq!(message, "policy");

    let allowed_delivery = delivery(settings(&v3_raw()), vec![reply(200, V3_ALLOW)]);
    let mut data =
        request_data(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#);
    run(
        &allowed_delivery,
        &mut data,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("the answer allows the turn");
}
