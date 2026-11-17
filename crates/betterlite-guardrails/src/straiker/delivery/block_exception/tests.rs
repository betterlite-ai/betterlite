#![cfg(test)]

use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use crate::straiker::delivery::fixtures::{
    Fakes, V1_BLOCK, V3_BLOCK, delivery, inputs, raw, reply, request_data, run, settings, v3_raw,
};

#[tokio::test]
async fn a_request_block_raises_the_guardrail() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_BLOCK)]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the answer blocks");
    let GuardrailFailure::GuardrailRaised {
        guardrail_name,
        message,
        wrap_default_message,
        status_code,
        blocked_content,
    } = failure
    else {
        panic!("expected a raised block");
    };
    assert_eq!(guardrail_name.as_deref(), Some("straiker"));
    assert_eq!(message, "policy");
    assert!(!wrap_default_message);
    assert_eq!(status_code, 400);
    assert!(blocked_content);
}

#[tokio::test]
async fn a_request_block_uses_the_configured_guardrail_name() {
    let fakes = Fakes::default();
    let mut args = raw();
    args.name = "my-straiker".to_owned();
    let handler = delivery(settings(&args), vec![reply(200, V1_BLOCK)]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the answer blocks");
    let GuardrailFailure::GuardrailRaised { guardrail_name, .. } = failure else {
        panic!("expected a raised block");
    };
    assert_eq!(guardrail_name.as_deref(), Some("my-straiker"));
}

#[tokio::test]
async fn a_block_without_a_reason_names_the_default_message() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&raw()),
        vec![reply(200, r#"{"action":"BLOCKED"}"#)],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the answer blocks");
    let GuardrailFailure::GuardrailRaised { message, .. } = failure else {
        panic!("expected a raised block");
    };
    assert_eq!(message, "Content violates policy");
}

#[tokio::test]
async fn a_v3_block_raises_the_guardrail_with_its_message() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&v3_raw()), vec![reply(200, V3_BLOCK)]);
    let mut data =
        request_data(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the answer blocks");
    let GuardrailFailure::GuardrailRaised {
        guardrail_name,
        message,
        blocked_content,
        ..
    } = failure
    else {
        panic!("expected a raised block");
    };
    assert_eq!(guardrail_name.as_deref(), Some("straiker"));
    assert_eq!(message, "policy");
    assert!(blocked_content);
}

#[tokio::test]
async fn a_response_block_raises_a_modify_response() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_BLOCK)]);
    let mut data = request_data(r#"{"model":"gpt-4o","response":{"id":"r1"}}"#);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        inputs(&["answer"]),
        GuardrailInputType::Response,
        &call,
    )
    .await
    .expect_err("the answer blocks");
    let GuardrailFailure::ModifyResponse(modify) = failure else {
        panic!("expected a modify-response block");
    };
    let ModifyResponseFailure {
        message,
        model,
        request_data: carried,
        guardrail_name,
        detection_info,
        original_response,
    } = *modify;
    assert_eq!(message, "policy");
    assert_eq!(model, "gpt-4o");
    assert_eq!(guardrail_name.as_deref(), Some("straiker"));
    assert!(detection_info.is_empty());
    assert_eq!(carried, data);
    assert_eq!(original_response, data.get("response").cloned());
}

#[rstest]
#[case::absent(r#"{"response":{"id":"r1"}}"#)]
#[case::not_text(r#"{"model":7,"response":{"id":"r1"}}"#)]
#[tokio::test]
async fn a_response_block_without_a_usable_model_names_unknown(#[case] body: &str) {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_BLOCK)]);
    let mut data = request_data(body);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        inputs(&["answer"]),
        GuardrailInputType::Response,
        &call,
    )
    .await
    .expect_err("the answer blocks");
    let GuardrailFailure::ModifyResponse(modify) = failure else {
        panic!("expected a modify-response block");
    };
    assert_eq!(modify.model, "unknown");
    assert_eq!(modify.request_data, data);
}
