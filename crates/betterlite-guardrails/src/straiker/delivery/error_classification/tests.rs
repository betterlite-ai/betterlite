#![cfg(test)]

use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::straiker::delivery::fixtures::{
    Fakes, Reply, caps, delivery, inputs, network_error, raw, reply, request_data, run, sent,
    settings,
};
use crate::straiker::delivery::settings::StraikerSettings;
use crate::straiker_webhook::failure::WebhookFailure;

fn fail_open() -> StraikerSettings {
    let mut args = raw();
    args.unreachable_fallback = "fail_open".to_owned();
    settings(&args)
}

#[rstest]
#[case::request_timeout(408_u16)]
#[case::too_many_requests(429_u16)]
#[case::server_error(500_u16)]
#[case::bad_gateway(502_u16)]
#[case::unavailable(503_u16)]
#[case::gateway_timeout(504_u16)]
#[tokio::test]
async fn a_retryable_status_is_retried_until_the_last_attempt(#[case] status: u16) {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(status, "boom"); 3]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the status never clears");
    assert_eq!(sent(&handler).len(), 3);
    assert_eq!(caps(&handler).len(), 2);
}

#[rstest]
#[case::bad_request(400_u16)]
#[case::unauthorized(401_u16)]
#[case::forbidden(403_u16)]
#[case::not_found(404_u16)]
#[case::unprocessable(422_u16)]
#[case::not_implemented(501_u16)]
#[tokio::test]
async fn a_permanent_status_is_not_retried(#[case] status: u16) {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(status, "boom"); 3]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the status is permanent");
    assert_eq!(sent(&handler).len(), 1);
    assert!(caps(&handler).is_empty());
}

#[rstest]
#[case::bad_gateway(502_u16)]
#[case::unavailable(503_u16)]
#[case::gateway_timeout(504_u16)]
#[tokio::test]
async fn an_unreachable_status_fails_open(#[case] status: u16) {
    let fakes = Fakes::default();
    let handler = delivery(fail_open(), vec![reply(status, "down"); 3]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let answer = run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an unreachable endpoint fails open");
    assert_eq!(answer, inputs(&["hi"]));
    assert_eq!(sent(&handler).len(), 3);
}

#[rstest]
#[case::request_timeout(408_u16)]
#[case::too_many_requests(429_u16)]
#[case::server_error(500_u16)]
#[tokio::test]
async fn a_retryable_status_that_answered_blocks_even_when_open(#[case] status: u16) {
    let fakes = Fakes::default();
    let handler = delivery(fail_open(), vec![reply(status, "boom"); 3]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the endpoint answered, so it is reachable");
    assert_eq!(sent(&handler).len(), 3);
}

#[tokio::test]
async fn a_network_error_is_retryable_and_unreachable() {
    let fakes = Fakes::default();
    let handler = delivery(fail_open(), vec![network_error(); 3]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let answer = run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an unreachable endpoint fails open");
    assert_eq!(answer, inputs(&["hi"]));
    assert_eq!(sent(&handler).len(), 3);
}

#[tokio::test]
async fn a_non_retryable_network_failure_is_permanent_but_unreachable() {
    let fakes = Fakes::default();
    let handler = delivery(
        fail_open(),
        vec![Reply::Failure(WebhookFailure {
            message: "TLS handshake failed".to_owned(),
            is_unreachable: true,
            retryable: false,
        })],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let answer = run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an unreachable endpoint fails open");
    assert_eq!(answer, inputs(&["hi"]));
    assert_eq!(sent(&handler).len(), 1);
    assert!(caps(&handler).is_empty());
}

#[tokio::test]
async fn a_200_body_that_is_not_json_is_permanent() {
    let fakes = Fakes::default();
    let handler = delivery(fail_open(), vec![reply(200, "not json"); 3]);
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
    .expect_err("a schema error is permanent");
    assert_eq!(sent(&handler).len(), 1);
    assert!(caps(&handler).is_empty());
    let GuardrailFailure::GuardrailRaised { message, .. } = failure else {
        panic!("expected a block");
    };
    assert!(message.contains("invalid response schema"), "got {message}");
}

#[tokio::test]
async fn the_status_and_its_text_are_named() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(400, "denied")]);
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
    .expect_err("the request is rejected");
    assert_eq!(
        failure.to_string(),
        "Straiker detection unavailable: HTTP 400: denied"
    );
}

#[tokio::test]
async fn the_status_text_is_cut_to_two_hundred_characters() {
    let fakes = Fakes::default();
    let long = "a".repeat(300);
    let handler = delivery(settings(&raw()), vec![reply(500, &long); 3]);
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
    .expect_err("the server keeps failing");
    assert_eq!(
        failure.to_string(),
        format!(
            "Straiker detection unavailable: HTTP 500: {}",
            "a".repeat(200)
        )
    );
}
