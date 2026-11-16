#![cfg(test)]

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::straiker::delivery::fixtures::{
    Fakes, V1_INTERVENE, api_inputs, delivery, inputs, raw, reply, request_data, run, settings,
    v3_raw,
};

const RESPONSE_DATA: &str = r#"{"model":"gpt-4o","response":{"id":"r1"}}"#;

fn raised(failure: GuardrailFailure) -> (String, bool) {
    let GuardrailFailure::GuardrailRaised {
        message,
        blocked_content,
        ..
    } = failure
    else {
        panic!("expected a raised block");
    };
    (message, blocked_content)
}

#[tokio::test]
async fn a_v1_intervention_replaces_the_texts() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_INTERVENE)]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let answer = run(
        &handler,
        &mut data,
        inputs(&["original"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("the intervention rewrites the texts");
    assert_eq!(answer.texts, Some(vec!["rewritten".to_owned()]));
}

#[tokio::test]
async fn an_intervention_keeps_every_other_input() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_INTERVENE)]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let original = api_inputs(
        r#"{"texts":["original"],"images":["img"],"model":"gpt-4o","stream_holdback_chars":[5]}"#,
    );
    let answer = run(
        &handler,
        &mut data,
        original.clone(),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("the intervention rewrites the texts");
    let mut expected = original;
    expected.texts = Some(vec!["rewritten".to_owned()]);
    assert_eq!(answer, expected);
}

#[tokio::test]
async fn a_v1_intervention_without_texts_blocks_the_turn() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&raw()),
        vec![reply(200, r#"{"action":"GUARDRAIL_INTERVENED"}"#)],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        inputs(&["original"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("there is nothing to rewrite with");
    assert_eq!(
        raised(failure),
        ("Content violates policy".to_owned(), true)
    );
}

#[tokio::test]
async fn an_empty_intervention_clears_the_texts() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&raw()),
        vec![reply(
            200,
            r#"{"action":"GUARDRAIL_INTERVENED","texts":[]}"#,
        )],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    let answer = run(
        &handler,
        &mut data,
        inputs(&["original"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("the intervention replaces the texts");
    assert_eq!(answer.texts, Some(Vec::new()));
}

#[tokio::test]
async fn a_streamed_response_intervention_blocks_the_turn() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_INTERVENE)]);
    let mut data =
        request_data(r#"{"model":"gpt-4o","stream":true,"response":{"id":"r1","choices":[]}}"#);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        inputs(&["answer"]),
        GuardrailInputType::Response,
        &call,
    )
    .await
    .expect_err("a sent stream cannot be rewritten");
    assert_eq!(
        raised(failure),
        ("Content violates policy".to_owned(), true)
    );
}

#[tokio::test]
async fn a_streamed_proxy_body_intervention_blocks_the_turn() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_INTERVENE)]);
    let mut data = request_data(
        r#"{"model":"gpt-4o","proxy_server_request":{"body":{"stream":true}},"response":{"id":"r1"}}"#,
    );
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        inputs(&["answer"]),
        GuardrailInputType::Response,
        &call,
    )
    .await
    .expect_err("the proxied request streams");
    assert_eq!(
        raised(failure),
        ("Content violates policy".to_owned(), true)
    );
}

#[tokio::test]
async fn a_streamed_request_intervention_still_rewrites_the_texts() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_INTERVENE)]);
    let mut data = request_data(r#"{"model":"gpt-4o","stream":true}"#);
    let call = fakes.call(Some("completion"), None);
    let answer = run(
        &handler,
        &mut data,
        inputs(&["original"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("a streamed request is rewritten before it is sent");
    assert_eq!(answer.texts, Some(vec!["rewritten".to_owned()]));
}

#[tokio::test]
async fn an_unstreamed_response_intervention_rewrites_the_texts() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_INTERVENE)]);
    let mut data = request_data(RESPONSE_DATA);
    let call = fakes.call(Some("completion"), None);
    let answer = run(
        &handler,
        &mut data,
        inputs(&["answer"]),
        GuardrailInputType::Response,
        &call,
    )
    .await
    .expect("an assembled answer is rewritten");
    assert_eq!(answer.texts, Some(vec!["rewritten".to_owned()]));
}

#[tokio::test]
async fn a_v3_answer_never_rewrites_the_texts() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&v3_raw()),
        vec![reply(
            200,
            r#"{"straiker":{"action":"allow"},"action":"GUARDRAIL_INTERVENED","texts":["rewritten"]}"#,
        )],
    );
    let mut data =
        request_data(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#);
    let call = fakes.call(Some("completion"), None);
    let original = inputs(&["original"]);
    let answer = run(
        &handler,
        &mut data,
        original.clone(),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("the v3 answer allows the turn");
    assert_eq!(answer, original);
}
