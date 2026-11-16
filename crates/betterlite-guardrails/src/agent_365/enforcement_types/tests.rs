#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{
    BlockedDetail, DefenderResult, EvaluateResponseError, EvaluateVerdict, UnavailableDetail,
    Verdict, decode_evaluate_response,
};
use crate::agent_365_tokens::DEFENDER_STATUS_EVALUATED;

fn allowed_body() -> Vec<u8> {
    br#"{"allowed":true,"defender":{"status":"Evaluated","verdict":"Allow","message":null},"correlationId":"corr-1"}"#.to_vec()
}

fn decode(body: &str) -> Result<EvaluateVerdict, EvaluateResponseError> {
    decode_evaluate_response(body.as_bytes())
}

#[test]
fn decodes_an_evaluated_allow() {
    let verdict = decode_evaluate_response(&allowed_body()).unwrap();
    assert!(verdict.allowed);
    assert_eq!(
        verdict.defender,
        DefenderResult {
            status: Some("Evaluated".to_owned()),
            verdict: Some("Allow".to_owned()),
            message: None,
        }
    );
    assert_eq!(verdict.correlation_id.as_deref(), Some("corr-1"));
    assert_eq!(
        verdict.defender.status.as_deref(),
        Some(DEFENDER_STATUS_EVALUATED)
    );
}

#[test]
fn decodes_a_blocked_verdict_with_its_message() {
    let verdict = decode(
        r#"{"allowed":false,"defender":{"status":"Evaluated","message":"Prompt injection detected"},"correlationId":"corr-2"}"#,
    )
    .unwrap();
    assert!(!verdict.allowed);
    assert_eq!(
        verdict.defender.message.as_deref(),
        Some("Prompt injection detected")
    );
    assert_eq!(
        verdict.blocked_message("get_weather"),
        "Prompt injection detected"
    );
}

#[test]
fn treats_a_non_object_defender_as_missing() {
    let verdict =
        decode(r#"{"allowed":true,"defender":"Evaluated","correlationId":"corr-1"}"#).unwrap();
    assert_eq!(verdict.defender, DefenderResult::default());
    assert_ne!(
        verdict.defender.status.as_deref(),
        Some(DEFENDER_STATUS_EVALUATED)
    );
    assert_eq!(verdict.correlation_id.as_deref(), Some("corr-1"));
}

#[test]
fn ignores_non_string_defender_fields_and_correlation_id() {
    let verdict = decode(
        r#"{"allowed":true,"defender":{"status":7,"verdict":false,"message":[]},"correlationId":9}"#,
    )
    .unwrap();
    assert_eq!(verdict.defender, DefenderResult::default());
    assert!(verdict.correlation_id.is_none());
    assert_eq!(
        verdict.blocked_message("run_sql"),
        "Invocation of 'run_sql' is blocked by Microsoft Threat Detection policies configured by your administrator."
    );
}

#[test]
fn rejects_a_non_json_body() {
    let error = decode("not json").unwrap_err();
    assert_eq!(
        error.to_string(),
        "the Agent 365 endpoint returned a non-JSON body"
    );
    assert!(matches!(error, EvaluateResponseError::NonJsonBody { .. }));
    let truncated = decode(r#"{"allowed":true"#).unwrap_err();
    assert!(matches!(
        truncated,
        EvaluateResponseError::NonJsonBody { .. }
    ));
}

fn assert_non_object(body: &str) {
    let error = decode(body).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the Agent 365 endpoint returned a non-object JSON body"
    );
    assert!(matches!(error, EvaluateResponseError::NonObjectBody));
}

fn assert_missing_allowed(body: &str) {
    let error = decode(body).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the Agent 365 endpoint returned a verdict without a boolean 'allowed' field"
    );
    assert!(matches!(error, EvaluateResponseError::MissingAllowed));
}

#[test]
fn rejects_a_non_object_json_body() {
    assert_non_object("[]");
    assert_non_object(r#""allowed""#);
    assert_non_object("null");
    assert_non_object("5");
}

#[test]
fn rejects_an_array_that_lists_the_verdict_fields_in_order() {
    assert_non_object(r#"[true, {"status":"Evaluated"}]"#);
    assert_non_object(r#"[true, {"status":"Evaluated"}, "corr-1"]"#);
}

#[test]
fn reads_an_array_defender_as_an_empty_one() {
    let verdict = decode(r#"{"allowed":true,"defender":["Evaluated",null,null]}"#).unwrap();
    assert!(verdict.allowed);
    assert_eq!(verdict.defender, DefenderResult::default());
    assert_ne!(
        verdict.defender.status.as_deref(),
        Some(DEFENDER_STATUS_EVALUATED)
    );
    assert_eq!(
        verdict.unevaluated_reason(),
        "Microsoft Defender did not evaluate the call (defender.status=missing)"
    );
}

#[test]
fn takes_the_last_of_repeated_keys() {
    let verdict =
        decode(r#"{"allowed":false,"allowed":true,"defender":{"status":"Evaluated"}}"#).unwrap();
    assert!(verdict.allowed);
}

#[test]
fn rejects_a_missing_or_non_boolean_allowed() {
    assert_missing_allowed("{}");
    assert_missing_allowed(r#"{"defender":{"status":"Evaluated"}}"#);
    assert_missing_allowed(r#"{"allowed":1}"#);
    assert_missing_allowed(r#"{"allowed":null}"#);
    assert_missing_allowed(r#"{"allowed":"true"}"#);
}

#[test]
fn an_empty_allowed_false_is_still_a_verdict() {
    let verdict = decode(r#"{"allowed":false}"#).unwrap();
    assert!(!verdict.allowed);
    assert_eq!(verdict.defender, DefenderResult::default());
}

#[test]
fn the_unevaluated_reason_names_a_missing_status() {
    let verdict = decode(r#"{"allowed":true}"#).unwrap();
    assert_eq!(
        verdict.unevaluated_reason(),
        "Microsoft Defender did not evaluate the call (defender.status=missing)"
    );
}

#[test]
fn the_unevaluated_reason_uses_an_empty_status_as_given() {
    let verdict = decode(r#"{"allowed":true,"defender":{"status":""}}"#).unwrap();
    assert_eq!(
        verdict.unevaluated_reason(),
        "Microsoft Defender did not evaluate the call (defender.status=)"
    );
}

#[test]
fn the_blocked_message_falls_back_to_the_default() {
    let verdict = decode(r#"{"allowed":false,"defender":{"status":"Evaluated"}}"#).unwrap();
    assert_eq!(
        verdict.blocked_message("run_sql"),
        "Invocation of 'run_sql' is blocked by Microsoft Threat Detection policies configured by your administrator."
    );
}

#[test]
fn the_unavailable_detail_writes_error_message_and_tool() {
    let detail = UnavailableDetail {
        error: "Agent 365 guardrail could not authorize the tool call".to_owned(),
        message: "blocked".to_owned(),
        tool: "get_weather".to_owned(),
    }
    .detail();
    let keys: Vec<&str> = detail.keys().map(String::as_str).collect();
    assert_eq!(keys, ["error", "message", "tool"]);
    assert_eq!(
        detail["tool"],
        GuardrailValue::Text("get_weather".to_owned())
    );
}

#[test]
fn the_blocked_detail_always_writes_the_correlation_id() {
    let detail = BlockedDetail {
        error: "Blocked by Microsoft Defender".to_owned(),
        message: "blocked".to_owned(),
        tool: "get_weather".to_owned(),
        correlation_id: None,
    }
    .detail();
    let keys: Vec<&str> = detail.keys().map(String::as_str).collect();
    assert_eq!(keys, ["error", "message", "tool", "correlation_id"]);
    assert_eq!(detail["correlation_id"], GuardrailValue::Null);
    let present = BlockedDetail {
        error: "Blocked by Microsoft Defender".to_owned(),
        message: "blocked".to_owned(),
        tool: "get_weather".to_owned(),
        correlation_id: Some("corr-9".to_owned()),
    }
    .detail();
    assert_eq!(
        present["correlation_id"],
        GuardrailValue::Text("corr-9".to_owned())
    );
}

#[test]
fn verdict_names_are_the_record_strings() {
    assert_eq!(Verdict::Allow.name(), "Allow");
    assert_eq!(Verdict::Block.name(), "Block");
    assert_eq!(Verdict::Rejected.name(), "Rejected");
    assert_eq!(Verdict::Throttled.name(), "Throttled");
    assert_eq!(Verdict::Unscanned.name(), "Unscanned");
    assert_eq!(Verdict::Unavailable.name(), "Unavailable");
}

// What a corpus entry decoded to, so the replay compares one map instead of six matches
fn describe(outcome: &Result<EvaluateVerdict, EvaluateResponseError>) -> &'static str {
    match outcome {
        Ok(verdict) if verdict.allowed => "allowed",
        Ok(_) => "blocked",
        Err(EvaluateResponseError::NonJsonBody { .. }) => "non-JSON body",
        Err(EvaluateResponseError::NonObjectBody) => "non-object body",
        Err(EvaluateResponseError::MissingAllowed) => "missing allowed",
    }
}

#[test]
fn replays_the_agent_365_evaluate_response_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/agent_365_evaluate_response");
    let entries: Vec<_> = fs::read_dir(corpus).unwrap().collect();
    assert!(!entries.is_empty());
    let outcomes: BTreeMap<String, &'static str> = entries
        .into_iter()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let bytes = fs::read(&path).unwrap();
            (name, describe(&decode_evaluate_response(&bytes)))
        })
        .collect();
    assert_eq!(outcomes.len(), 6);
    assert_eq!(
        outcomes,
        BTreeMap::from([
            ("allowed.json".to_owned(), "allowed"),
            ("array_verdict.json".to_owned(), "non-object body"),
            ("blocked.json".to_owned(), "blocked"),
            ("missing_allowed.json".to_owned(), "missing allowed"),
            ("non_object.json".to_owned(), "non-object body"),
            ("truncated.json".to_owned(), "non-JSON body"),
        ])
    );
}
