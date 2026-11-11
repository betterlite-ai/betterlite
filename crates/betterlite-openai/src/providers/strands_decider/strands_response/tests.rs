#![cfg(test)]

use betterlite_models::decisions::ir;
use betterlite_models::decisions::{DecisionsJson, Extra};
use rstest::rstest;
use serde_json::Number;

use super::StrandsDeciderDecisionsConfig;
use crate::decisions::transformation::provider_config::{
    DecisionsProviderConfig as _, ParseResponseError,
};

fn predicate_request() -> ir::DecisionRequest {
    ir::DecisionRequest {
        input: ir::DecisionInput::State(DecisionsJson::Text("the state".to_owned())),
        questions: vec![ir::DecisionQuestion::Predicate(ir::PredicateQuestion {
            name: Some("q".to_owned()),
            instructions: Some(DecisionsJson::Text("decide".to_owned())),
            criteria: None,
            extra: Extra::default(),
        })],
        safety_identifier: None,
    }
}

#[rstest]
#[case::half("0.5")]
#[case::quarter("0.25")]
fn test_non_runtime_fallthrough_parses_a_systemone_response(#[case] probability: &str) {
    let body = format!(
        r#"{{"model": "m", "answers": {{"q": {{"type": "noul", "noul": {probability}}}}}, "usage": {{"input_tokens": 3, "output_tokens": 4}}}}"#
    );
    let config = StrandsDeciderDecisionsConfig::default();
    let response = config
        .transform_decisions_response(
            "m",
            "strands_decider",
            body.as_bytes(),
            &predicate_request(),
        )
        .expect("a parsed systemone response");
    assert_eq!(response.model.as_deref(), Some("m"));
    assert_eq!(response.answers.len(), 1);
    match &response.answers[0] {
        ir::DecisionAnswer::Predicate {
            probability: parsed,
            ..
        } => {
            assert_eq!(parsed.to_string(), probability);
        }
        other @ (ir::DecisionAnswer::Choice { .. }
        | ir::DecisionAnswer::Score { .. }
        | ir::DecisionAnswer::Refusal) => panic!("a predicate answer, got {other:?}"),
    }
    assert_eq!(response.usage.input_tokens.0, Number::from(3_u8));
    assert_eq!(response.usage.output_tokens.0, Number::from(4_u8));
}

#[rstest]
#[case::unstructured_error(br#"{"error": "unstructured"}"#)]
#[case::not_an_object(b"[]")]
#[case::wrong_answers_type(br#"{"answers": "text"}"#)]
fn test_non_runtime_fallthrough_wraps_an_unexpected_response_at_status_500(#[case] body: &[u8]) {
    let config = StrandsDeciderDecisionsConfig::default();
    let result =
        config.transform_decisions_response("m", "strands_decider", body, &predicate_request());
    match result {
        Err(ParseResponseError::Invalid(error)) => {
            assert_eq!(error.status_code, 500);
            assert!(
                error.message.starts_with(
                    "Decisions provider 'strands_decider' returned an unexpected response"
                ),
                "{}",
                error.message
            );
        }
        _ => panic!("a wrapped unexpected response"),
    }
}

#[test]
fn non_runtime_fallthrough_reports_unreadable_json_as_malformed() {
    let config = StrandsDeciderDecisionsConfig::default();
    let result =
        config.transform_decisions_response("m", "strands_decider", b"{", &predicate_request());
    assert!(matches!(result, Err(ParseResponseError::Malformed(_))));
}
