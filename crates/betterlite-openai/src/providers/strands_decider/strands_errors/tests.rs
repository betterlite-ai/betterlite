#![cfg(test)]

use betterlite_models::decisions::ir;
use betterlite_models::decisions::{DecisionsJson, Extra};
use rstest::rstest;

use super::StrandsDeciderDecisionsConfig;
use super::response::{runtime_error, runtime_error_status};
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
#[case::bad_request("bad_request", 400)]
#[case::invalid_request("invalid_request", 400)]
#[case::loading("loading", 503)]
#[case::inference_error("inference_error", 500)]
#[case::unknown_code("new_code", 500)]
#[case::other_casing("Bad_Request", 500)]
#[case::empty("", 500)]
fn test_error_status_mapping_translates_only_the_known_codes(
    #[case] code: &str,
    #[case] status: u16,
) {
    assert_eq!(runtime_error_status(code), status);
}

#[rstest]
#[case::bad_request("bad_request", 400)]
#[case::invalid_request("invalid_request", 400)]
#[case::loading("loading", 503)]
#[case::unknown_code("new_code", 500)]
fn test_runtime_error_response_raises_with_the_code_and_message(
    #[case] code: &str,
    #[case] status: u16,
) {
    let body = format!(r#"{{"error": {{"code": "{code}", "message": "at most 16 questions"}}}}"#);
    let config = StrandsDeciderDecisionsConfig::default();
    let result = config.transform_decisions_response(
        "m",
        "strands_decider",
        body.as_bytes(),
        &predicate_request(),
    );
    match result {
        Err(ParseResponseError::Invalid(error)) => {
            assert_eq!(error.status_code, status);
            assert!(error.message.contains(code), "{}", error.message);
            assert!(
                error.message.contains("at most 16 questions"),
                "{}",
                error.message
            );
        }
        _ => panic!("a mapped runtime error"),
    }
}

#[test]
fn runtime_error_response_keeps_a_reply_with_answers_out_of_the_error_envelope() {
    assert!(runtime_error(br#"{"error": {"code": "bad_request"}}"#).is_some());
    assert!(runtime_error(br#"{"error": {"code": "bad_request"}, "answers": null}"#).is_some());
    assert!(runtime_error(br#"{"error": {"code": "bad_request"}, "answers": {}}"#).is_none());
    assert!(runtime_error(br#"{"error": {"code": "bad_request"}, "answers": 0}"#).is_none());
}

#[test]
fn runtime_error_response_ignores_a_reply_without_a_code_and_message() {
    assert!(runtime_error(br#"{"error": "unstructured"}"#).is_none());
    assert!(runtime_error(br#"{"error": {}}"#).is_none());
    assert!(runtime_error(br#"{"error": {"code": 5}}"#).is_none());
    assert!(runtime_error(br#"{"error": {"code": "x", "message": null}}"#).is_none());
    assert!(runtime_error(br#"{"error": {"code": "x"}, "extra": true}"#).is_some());
}

#[test]
fn runtime_error_response_surfaces_malformed_json_from_the_parent_parser() {
    let config = StrandsDeciderDecisionsConfig::default();
    let result = config.transform_decisions_response(
        "m",
        "strands_decider",
        b"not json",
        &predicate_request(),
    );
    assert!(matches!(result, Err(ParseResponseError::Malformed(_))));
}
