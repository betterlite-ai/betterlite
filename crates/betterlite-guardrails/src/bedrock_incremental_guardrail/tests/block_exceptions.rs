#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::support::{
    GUARDRAIL_NAME, Harness, blocking, guardrail_without_block_exception, inputs,
};
use crate::bedrock_decisions::assessments::VIOLATED_POLICY;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;

#[tokio::test]
async fn a_blocking_reply_raises_the_policy_error() {
    let mut harness = Harness::new(vec![blocking()]);

    let failure = harness
        .apply(inputs(&["hello"]), GuardrailInputType::Request)
        .await
        .expect_err("the guardrail blocks the text");

    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("a block is an http failure with a structured detail");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(VIOLATED_POLICY.to_owned()))
    );
    assert_eq!(
        detail.get("bedrock_guardrail_response"),
        Some(&GuardrailValue::Text("blocked text".to_owned()))
    );
}

#[tokio::test]
async fn a_block_with_exceptions_disabled_becomes_a_modify_response() {
    let mut harness = Harness::new(vec![blocking()]);
    harness.guardrail = guardrail_without_block_exception();

    let failure = harness
        .apply(inputs(&["hello"]), GuardrailInputType::Request)
        .await
        .expect_err("the guardrail blocks the text");

    let GuardrailFailure::ModifyResponse(modified) = failure else {
        panic!("a block without exceptions is a modify response");
    };
    assert_eq!(modified.message, "blocked text");
    assert_eq!(modified.guardrail_name.as_deref(), Some(GUARDRAIL_NAME));
}
