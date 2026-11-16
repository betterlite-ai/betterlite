#![cfg(test)]

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::straiker::delivery::fixtures::{
    Fakes, api_inputs, delivery, network_error, raw, reply, request_data, run, settings, v3_raw,
};
use crate::straiker::delivery::settings::StraikerSettings;

fn fallback_settings(fallback: &str, fail_on_error: bool) -> StraikerSettings {
    let mut args = raw();
    args.unreachable_fallback = fallback.to_owned();
    args.fail_on_error = fail_on_error;
    settings(&args)
}

fn untouched() -> GuardrailApiInputs {
    api_inputs(r#"{"texts":["hi"],"images":["img"],"model":"gpt-4o"}"#)
}

#[tokio::test]
async fn a_closed_fallback_blocks_an_unreachable_endpoint() {
    let fakes = Fakes::default();
    let handler = delivery(
        fallback_settings("fail_closed", true),
        vec![network_error(), network_error(), network_error()],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        untouched(),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the endpoint is unreachable");
    let GuardrailFailure::GuardrailRaised {
        message,
        blocked_content,
        status_code,
        wrap_default_message,
        ..
    } = failure
    else {
        panic!("expected a raised block");
    };
    assert_eq!(
        message,
        "Straiker detection unavailable: ConnectError: refused"
    );
    assert!(!blocked_content);
    assert_eq!(status_code, 400);
    assert!(!wrap_default_message);
}

#[tokio::test]
async fn a_closed_fallback_blocks_an_unreadable_answer() {
    let fakes = Fakes::default();
    let handler = delivery(
        fallback_settings("fail_closed", true),
        vec![reply(200, "not json")],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        untouched(),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the answer is not JSON");
    let GuardrailFailure::GuardrailRaised {
        message,
        blocked_content,
        ..
    } = failure
    else {
        panic!("expected a raised block");
    };
    assert!(message.starts_with("Straiker detection unavailable: invalid response schema"));
    assert!(!blocked_content);
}

#[tokio::test]
async fn an_open_fallback_returns_the_inputs_of_an_unreachable_endpoint() {
    let fakes = Fakes::default();
    let handler = delivery(
        fallback_settings("fail_open", true),
        vec![network_error(), network_error(), network_error()],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let original = untouched();
    let answer = run(
        &handler,
        &mut data,
        original.clone(),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an unreachable endpoint falls through");
    assert_eq!(answer, original);
}

#[tokio::test]
async fn an_open_fallback_still_blocks_an_unreadable_answer() {
    let fakes = Fakes::default();
    let handler = delivery(
        fallback_settings("fail_open", true),
        vec![reply(200, "not json")],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        untouched(),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("a reachable endpoint that answered badly is not a fallthrough");
}

#[tokio::test]
async fn an_open_fallback_still_blocks_a_permanent_status() {
    let fakes = Fakes::default();
    let handler = delivery(
        fallback_settings("fail_open", true),
        vec![reply(400, "denied")],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        untouched(),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the endpoint answered");
}

#[tokio::test]
async fn fail_on_error_off_returns_the_inputs_after_a_rejection() {
    let fakes = Fakes::default();
    let handler = delivery(
        fallback_settings("fail_closed", false),
        vec![reply(400, "denied")],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let original = untouched();
    let answer = run(
        &handler,
        &mut data,
        original.clone(),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("errors never block");
    assert_eq!(answer, original);
}

#[tokio::test]
async fn fail_on_error_off_returns_the_inputs_after_an_unreachable_endpoint() {
    let fakes = Fakes::default();
    let handler = delivery(
        fallback_settings("fail_closed", false),
        vec![network_error(), network_error(), network_error()],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let original = untouched();
    let answer = run(
        &handler,
        &mut data,
        original.clone(),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("errors never block");
    assert_eq!(answer, original);
}

#[tokio::test]
async fn a_response_phase_failure_becomes_a_modify_response() {
    let fakes = Fakes::default();
    let handler = delivery(
        fallback_settings("fail_closed", true),
        vec![reply(400, "denied")],
    );
    let mut data = request_data(r#"{"model":"gpt-4o","response":{"id":"r1"}}"#);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        untouched(),
        GuardrailInputType::Response,
        &call,
    )
    .await
    .expect_err("the answer is rejected");
    let GuardrailFailure::ModifyResponse(modify) = failure else {
        panic!("expected a modify-response block");
    };
    assert_eq!(
        modify.message,
        "Straiker detection unavailable: HTTP 400: denied"
    );
    assert_eq!(modify.model, "gpt-4o");
    assert_eq!(modify.original_response, data.get("response").cloned());
}

#[tokio::test]
async fn a_v3_failure_blocks_an_agent_key() {
    let fakes = Fakes::default();
    let mut args = v3_raw();
    args.unreachable_fallback = "fail_open".to_owned();
    let handler = delivery(settings(&args), vec![reply(200, "not json")]);
    let mut data =
        request_data(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        untouched(),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the v3 answer is unreadable and the endpoint answered");
}
