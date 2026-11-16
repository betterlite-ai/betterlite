#![cfg(test)]

use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::straiker::delivery::fixtures::{
    Fakes, V1_ALLOW, V1_BLOCK, capture_fields, delivery, inputs, raw, reply, request_data, run,
    sent, settings, v3_raw,
};
use crate::third_party_init_b::straiker::StraikerArgs;

fn verbose_raw() -> StraikerArgs {
    let mut args = raw();
    args.verbose = Some(true);
    args
}

fn value_of(fields: &[(String, String)], name: &str) -> String {
    fields
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

fn messages(fields: &[(String, String)]) -> Vec<String> {
    fields
        .iter()
        .filter(|(key, _)| key == "message")
        .map(|(_, value)| value.clone())
        .collect()
}

fn hidden(body: &str) -> String {
    format!(
        r#"{{"model":"gpt-4o","messages":[{{"role":"user","content":"hello"}}],"response":{body}}}"#
    )
}

#[tokio::test]
async fn the_verdict_is_recorded_beside_the_existing_hidden_params() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&verbose_raw()), vec![reply(200, V1_ALLOW)]);
    let mut data = request_data(&hidden(
        r#"{"id":"r1","_hidden_params":{"straiker":{"keep":"yes"},"other":1}}"#,
    ));
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    let expected = request_data(&hidden(
        r#"{"id":"r1","_hidden_params":{"straiker":{"keep":"yes","action":"NONE","turn_id":null},"other":1}}"#,
    ));
    assert_eq!(data, expected);
}

#[tokio::test]
async fn a_block_is_recorded_before_it_is_raised() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&verbose_raw()), vec![reply(200, V1_BLOCK)]);
    let mut data = request_data(&hidden(r#"{"id":"r1","_hidden_params":{}}"#));
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
    assert_eq!(failure.to_string(), "policy");
    let expected = request_data(&hidden(
        r#"{"id":"r1","_hidden_params":{"straiker":{"action":"BLOCKED","turn_id":"turn-1"}}}"#,
    ));
    assert_eq!(data, expected);
}

#[tokio::test]
async fn a_v3_verdict_takes_its_turn_id_from_the_body() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&{
            let mut args = v3_raw();
            args.verbose = Some(true);
            args
        }),
        vec![reply(
            200,
            r#"{"straiker":{"action":"allow"},"turn_id":"turn-9"}"#,
        )],
    );
    let mut data = request_data(&hidden(r#"{"id":"r1","_hidden_params":{}}"#));
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    let expected = request_data(&hidden(
        r#"{"id":"r1","_hidden_params":{"straiker":{"action":"NONE","turn_id":"turn-9"}}}"#,
    ));
    assert_eq!(data, expected);
}

#[tokio::test]
async fn verbose_off_records_nothing() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_ALLOW)]);
    let mut data = request_data(&hidden(r#"{"id":"r1","_hidden_params":{}}"#));
    let expected = request_data(&hidden(r#"{"id":"r1","_hidden_params":{}}"#));
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert_eq!(data, expected);
}

#[rstest]
#[case::no_response(r#"{"model":"gpt-4o"}"#)]
#[case::text_response(r#"{"model":"gpt-4o","response":"x"}"#)]
#[case::no_hidden(r#"{"model":"gpt-4o","response":{"id":"r1"}}"#)]
#[case::text_hidden(r#"{"model":"gpt-4o","response":{"_hidden_params":"x"}}"#)]
#[case::text_straiker(r#"{"model":"gpt-4o","response":{"_hidden_params":{"straiker":"x"}}}"#)]
#[tokio::test]
async fn an_unrecordable_answer_is_left_alone(#[case] body: &str) {
    let fakes = Fakes::default();
    let handler = delivery(settings(&verbose_raw()), vec![reply(200, V1_ALLOW)]);
    let mut data = request_data(body);
    let expected = request_data(body);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert_eq!(data, expected);
}

#[tokio::test]
async fn the_request_and_response_events_carry_their_bodies() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&verbose_raw()), vec![reply(200, V1_ALLOW)]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let (fields, _guard) = capture_fields();
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    let fields = fields.take();
    assert_eq!(
        messages(&fields),
        vec![
            "straiker.webhook_request".to_owned(),
            "straiker.webhook_response".to_owned()
        ]
    );
    let posts = sent(&handler);
    assert_eq!(value_of(&fields, "url"), posts[0].url);
    assert_eq!(value_of(&fields, "payload"), posts[0].body);
    assert_eq!(value_of(&fields, "bytes"), posts[0].body.len().to_string());
    assert_eq!(value_of(&fields, "status_code"), "200");
    assert_eq!(value_of(&fields, "body"), V1_ALLOW);
}

#[tokio::test]
async fn verbose_off_logs_no_webhook_event() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_ALLOW)]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let (fields, _guard) = capture_fields();
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert!(messages(&fields.take()).is_empty());
}

#[tokio::test]
async fn an_unreadable_answer_logs_no_response_event() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&verbose_raw()), vec![reply(200, "not json")]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let (fields, _guard) = capture_fields();
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the answer is not JSON");
    let fields = fields.take();
    assert_eq!(
        messages(&fields),
        vec![
            "straiker.webhook_request".to_owned(),
            "straiker.error".to_owned()
        ]
    );
    assert!(value_of(&fields, "status_code").is_empty());
}

#[tokio::test]
async fn the_error_event_names_the_phase_and_the_fallthrough() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(400, "denied")]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let (fields, _guard) = capture_fields();
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the request is rejected");
    let fields = fields.take();
    assert_eq!(messages(&fields), vec!["straiker.error".to_owned()]);
    assert_eq!(value_of(&fields, "input_type"), "request");
    assert_eq!(value_of(&fields, "error"), "HTTP 400: denied");
    assert_eq!(value_of(&fields, "fail_open"), "false");
}
