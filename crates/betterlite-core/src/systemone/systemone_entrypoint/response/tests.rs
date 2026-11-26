#![cfg(test)]

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::openai::DecisionResponse as OpenAiDecisionResponse;
use betterlite_models::decisions::systemone::DecisionsResponse;

use super::systemone_response;
use crate::api::decisions_send::format::DecisionsOutput;
use crate::decisions::DecisionsEntryError;

fn systemone_answer() -> DecisionsResponse {
    serde_json::from_str(
        r#"{"model":"model-1","answers":{"first":{"type":"noul","noul":0.9}},"usage":{"input_tokens":3,"output_tokens":5}}"#,
    )
    .expect("the System One answer parses")
}

fn openai_answer() -> OpenAiDecisionResponse {
    serde_json::from_str(
        r#"{"model":"model-1","answers":[{"type":"predicate","name":"first","probability":0.9}],"usage":{"input_tokens":3,"output_tokens":5,"total_tokens":8}}"#,
    )
    .expect("the OpenAI answer parses")
}

#[test]
fn returns_a_systemone_answer() {
    let answer = systemone_answer();
    let output = systemone_response(DecisionsOutput::SystemOne(answer.clone()))
        .expect("the System One answer passes");
    assert_eq!(output, answer);
}

#[test]
fn rejects_an_openai_answer() {
    let error = systemone_response(DecisionsOutput::OpenAi(openai_answer()))
        .expect_err("an OpenAI answer is not a System One answer");
    assert!(matches!(error, DecisionsEntryError::ResponseFormat { .. }));
    assert!(matches!(error.class(), ErrorClass::InternalServer(_)));
    assert_eq!(error.class().metadata().status_code, 500);
}
