#![cfg(test)]

use betterlite_config::sections::mock_response::MockResponse;
use betterlite_config::sections::mock_response::selection::MockPlan;
use betterlite_error::envelope::WireField;
use rstest::rstest;

use crate::mock::chat::{self, ChatAnswer};
use crate::rules::{RuleDecision, RuleError, RuleEvaluator, RulePhase, evaluate};

struct Fixed<'rules> {
    rules: &'rules [Option<RuleDecision>],
}

impl RuleEvaluator for Fixed<'_> {
    fn evaluate(&self, index: usize, _input: &str) -> Option<RuleDecision> {
        self.rules
            .get(index)
            .map(|rule| rule.clone().unwrap_or(RuleDecision::NotCallable))
    }
}

struct Unreachable;

impl RuleEvaluator for Unreachable {
    fn evaluate(&self, _index: usize, _input: &str) -> Option<RuleDecision> {
        panic!("the evaluator must not run for an absent input")
    }
}

#[rstest]
#[case::pre_true_allows(RulePhase::PreCall, Some("answer"), vec![Some(RuleDecision::Boolean(true))], false)]
#[case::pre_false_rejects(RulePhase::PreCall, Some("answer"), vec![Some(RuleDecision::Boolean(true)), Some(RuleDecision::Boolean(false))], true)]
#[case::pre_dict_false_passes(RulePhase::PreCall, Some("answer"), vec![Some(RuleDecision::Dict { decision: Some(false), message: None })], false)]
#[case::pre_not_callable_is_skipped(RulePhase::PreCall, Some("answer"), vec![None, Some(RuleDecision::Boolean(false))], true)]
#[case::pre_none_input_is_an_error(RulePhase::PreCall, None, vec![Some(RuleDecision::Boolean(true))], true)]
#[case::post_true_allows(RulePhase::PostCall, Some("answer"), vec![Some(RuleDecision::Boolean(true))], false)]
#[case::post_false_rejects(RulePhase::PostCall, Some("answer"), vec![Some(RuleDecision::Boolean(false))], true)]
#[case::post_dict_true_allows(RulePhase::PostCall, Some("answer"), vec![Some(RuleDecision::Dict { decision: Some(true), message: None })], false)]
#[case::post_dict_absent_decision_allows(RulePhase::PostCall, Some("answer"), vec![Some(RuleDecision::Dict { decision: None, message: None })], false)]
#[case::post_dict_false_rejects(RulePhase::PostCall, Some("answer"), vec![Some(RuleDecision::Dict { decision: Some(false), message: None })], true)]
#[case::post_not_callable_is_skipped(RulePhase::PostCall, Some("answer"), vec![None, Some(RuleDecision::Boolean(false))], true)]
#[case::post_none_input_allows(RulePhase::PostCall, None, vec![Some(RuleDecision::Boolean(false))], false)]
fn call_rules_tests_behavior(
    #[case] phase: RulePhase,
    #[case] input: Option<&str>,
    #[case] rules: Vec<Option<RuleDecision>>,
    #[case] rejected: bool,
) {
    let evaluator = Fixed { rules: &rules };
    assert_eq!(
        evaluate(phase, &evaluator, input, "gpt-4o").is_err(),
        rejected
    );
}

#[test]
fn post_call_without_an_input_skips_the_evaluator() {
    evaluate(RulePhase::PostCall, &Unreachable, None, "gpt-4o").unwrap();
}

#[test]
fn pre_call_without_an_input_is_an_error() {
    let error = evaluate(RulePhase::PreCall, &Unreachable, None, "gpt-4o").unwrap_err();
    assert!(matches!(error, RuleError::MissingInput));
}

#[test]
fn post_call_uses_the_message_a_dict_decision_names() {
    let rules = [Some(RuleDecision::Dict {
        decision: Some(false),
        message: Some(String::from("refused by policy")),
    })];
    let error = evaluate(
        RulePhase::PostCall,
        &Fixed { rules: &rules },
        Some("answer"),
        "gpt-4o",
    )
    .unwrap_err();
    let RuleError::Rejected {
        phase,
        model,
        message,
    } = error
    else {
        panic!("a rejecting rule answers with RuleError::Rejected");
    };
    assert_eq!(phase, RulePhase::PostCall);
    assert_eq!(model, "gpt-4o");
    assert_eq!(message, "refused by policy");
}

#[test]
fn post_call_falls_back_to_the_fixed_message() {
    let rules = [Some(RuleDecision::Boolean(false))];
    let error = evaluate(
        RulePhase::PostCall,
        &Fixed { rules: &rules },
        Some("answer"),
        "gpt-4o",
    )
    .unwrap_err();
    let RuleError::Rejected { message, .. } = error else {
        panic!("a rejecting rule answers with RuleError::Rejected");
    };
    assert_eq!(message, "LLM Response failed post-call-rule check");
}

#[test]
fn pre_call_carries_the_fixed_message_and_the_phase() {
    let rules = [Some(RuleDecision::Boolean(false))];
    let error = evaluate(
        RulePhase::PreCall,
        &Fixed { rules: &rules },
        Some("answer"),
        "gpt-4o",
    )
    .unwrap_err();
    let RuleError::Rejected { phase, message, .. } = error else {
        panic!("a rejecting rule answers with RuleError::Rejected");
    };
    assert_eq!(phase, RulePhase::PreCall);
    assert_eq!(message, "LLM Response failed post-call-rule check");
}

#[test]
fn a_rule_rejection_classifies_as_an_api_response_validation_error() {
    let rules = [Some(RuleDecision::Boolean(false))];
    let error = evaluate(
        RulePhase::PostCall,
        &Fixed { rules: &rules },
        Some("answer"),
        "gpt-4o",
    )
    .unwrap_err();
    let metadata = error.classify().metadata();
    assert_eq!(metadata.status_code, 500);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.code, "500");
    assert_eq!(metadata.model.as_deref(), Some("gpt-4o"));
    assert_eq!(metadata.provider.as_deref(), Some(""));
}

#[test]
fn a_mock_chat_answer_feeds_the_post_call_rules() {
    let response = MockResponse::Text(String::from("{\"answer\": 1}"));
    let plan = MockPlan {
        response: Some(&response),
        tool_calls: None,
        choice_count: None,
        prompt_tokens: None,
    };
    let ChatAnswer::Content(content) = chat::answer(&plan).unwrap() else {
        panic!("a text mock answers with content");
    };
    let reject = [Some(RuleDecision::Dict {
        decision: Some(false),
        message: None,
    })];
    let rejected = evaluate(
        RulePhase::PostCall,
        &Fixed { rules: &reject },
        Some(content),
        "gpt-4o",
    );
    drop(rejected.unwrap_err());
    let allow = [Some(RuleDecision::Dict {
        decision: None,
        message: None,
    })];
    evaluate(
        RulePhase::PostCall,
        &Fixed { rules: &allow },
        Some(content),
        "gpt-4o",
    )
    .unwrap();
}
