#![cfg(test)]

use std::io;

use rstest::rstest;

use super::{
    GuardrailPipeline, PipelineAction, PipelineExecutionResult, PipelineMode, PipelineStepResult,
    StepOutcome,
};

#[test]
fn accepts_the_default_block_and_allow() {
    let pipeline: GuardrailPipeline =
        serde_json::from_str(r#"{"mode": "pre_call", "steps": [{"guardrail": "pii"}]}"#).unwrap();
    assert_eq!(pipeline.mode, PipelineMode::PreCall);
    let step = &pipeline.steps[0];
    assert_eq!(step.guardrail, "pii");
    assert_eq!(step.on_fail, PipelineAction::Block);
    assert_eq!(step.on_pass, PipelineAction::Allow);
    assert_eq!(step.on_error, None);
    assert!(!step.pass_data);
    assert_eq!(step.modify_response_message, None);
}

#[test]
fn keeps_explicit_step_values() {
    let pipeline: GuardrailPipeline = serde_json::from_str(
        r#"{
            "mode": "post_call",
            "steps": [{
                "guardrail": "pii",
                "on_fail": "next",
                "on_pass": "modify_response",
                "on_error": "allow",
                "pass_data": true,
                "modify_response_message": "masked"
            }]
        }"#,
    )
    .unwrap();
    assert_eq!(pipeline.mode, PipelineMode::PostCall);
    let step = &pipeline.steps[0];
    assert_eq!(step.on_fail, PipelineAction::Next);
    assert_eq!(step.on_pass, PipelineAction::ModifyResponse);
    assert_eq!(step.on_error, Some(PipelineAction::Allow));
    assert!(step.pass_data);
    assert_eq!(step.modify_response_message.as_deref(), Some("masked"));
}

#[rstest]
#[case::on_fail(r#"{"mode": "pre_call", "steps": [{"guardrail": "pii", "on_fail": "deny"}]}"#)]
#[case::on_pass(r#"{"mode": "pre_call", "steps": [{"guardrail": "pii", "on_pass": "deny"}]}"#)]
#[case::on_error(r#"{"mode": "pre_call", "steps": [{"guardrail": "pii", "on_error": "deny"}]}"#)]
fn rejects_unknown_actions(#[case] source: &str) {
    let error = serde_json::from_str::<GuardrailPipeline>(source).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid action 'deny'. Must be one of: ['allow', 'block', 'modify_response', 'next']"
    );
}

#[test]
fn rejects_an_unknown_mode() {
    let error = serde_json::from_str::<GuardrailPipeline>(
        r#"{"mode": "during_call", "steps": [{"guardrail": "pii"}]}"#,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid mode 'during_call'. Must be one of: ['post_call', 'pre_call']"
    );
}

#[test]
fn rejects_an_empty_step_list() {
    let error = serde_json::from_str::<GuardrailPipeline>(r#"{"mode": "pre_call", "steps": []}"#)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "List should have at least 1 item after validation, not 0"
    );
}

#[test]
fn actions_and_modes_serialize_to_their_lowercase_words() {
    assert_eq!(
        serde_json::to_string(&PipelineAction::Allow).unwrap(),
        r#""allow""#
    );
    assert_eq!(
        serde_json::to_string(&PipelineAction::Block).unwrap(),
        r#""block""#
    );
    assert_eq!(
        serde_json::to_string(&PipelineAction::Next).unwrap(),
        r#""next""#
    );
    assert_eq!(
        serde_json::to_string(&PipelineAction::ModifyResponse).unwrap(),
        r#""modify_response""#
    );
    assert_eq!(
        serde_json::to_string(&PipelineMode::PreCall).unwrap(),
        r#""pre_call""#
    );
    assert_eq!(
        serde_json::to_string(&PipelineMode::PostCall).unwrap(),
        r#""post_call""#
    );
}

#[test]
fn a_pipeline_round_trips_through_serialization() {
    let pipeline: GuardrailPipeline = serde_json::from_str(
        r#"{"mode": "post_call", "steps": [{"guardrail": "pii", "on_error": "block"}]}"#,
    )
    .unwrap();
    let text = serde_json::to_string(&pipeline).unwrap();
    let restored: GuardrailPipeline = serde_json::from_str(&text).unwrap();
    assert_eq!(restored, pipeline);
}

#[test]
fn step_result_carries_the_outcome_and_action() {
    let step: PipelineStepResult = serde_json::from_str(
        r#"{"guardrail_name": "pii", "outcome": "fail", "action_taken": "block"}"#,
    )
    .unwrap();
    assert_eq!(step.outcome, StepOutcome::Fail);
    assert_eq!(step.action_taken, PipelineAction::Block);
    assert_eq!(step.modified_data, None);
    assert_eq!(step.error_detail, None);
    assert_eq!(step.duration_seconds, None);

    let error: PipelineStepResult = serde_json::from_str(
        r#"{"guardrail_name": "pii", "outcome": "error", "action_taken": "next", "error_detail": "timeout"}"#,
    )
    .unwrap();
    assert_eq!(error.outcome, StepOutcome::Error);
    assert_eq!(error.action_taken, PipelineAction::Next);
    assert_eq!(error.error_detail.as_deref(), Some("timeout"));
}

#[test]
fn execution_result_skips_the_original_exception_when_serializing() {
    let result = PipelineExecutionResult {
        terminal_action: PipelineAction::Block,
        step_results: Vec::new(),
        modified_data: None,
        error_message: Some("blocked".to_owned()),
        modify_response_message: None,
        original_exception: Some(Box::new(io::Error::other("boom"))),
    };
    let text = serde_json::to_string(&result).unwrap();
    assert!(!text.contains("original_exception"));
    assert!(text.contains(r#""error_message":"blocked""#));
    let restored: PipelineExecutionResult = serde_json::from_str(&text).unwrap();
    assert_eq!(restored.terminal_action, PipelineAction::Block);
    assert!(restored.original_exception.is_none());
}

#[test]
fn execution_result_defaults_its_optional_fields() {
    let parsed: PipelineExecutionResult =
        serde_json::from_str(r#"{"terminal_action": "allow", "step_results": []}"#).unwrap();
    assert_eq!(parsed.terminal_action, PipelineAction::Allow);
    assert!(parsed.step_results.is_empty());
    assert_eq!(parsed.modified_data, None);
    assert_eq!(parsed.error_message, None);
    assert_eq!(parsed.modify_response_message, None);
}
