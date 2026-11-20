use std::collections::BTreeMap;

use betterlite_http::proxy::transport_error::TransportError;
use serde_json::json;

use super::{
    Closed, GUARDRAIL_NAME, IDENTIFIER, Reply, Scripted, VERSION, anonymous_guardrail, blocking,
    expected, field_of, guardrail, guardrail_without_block_exception, has_field, input_scan,
    message, records_of, run, usage_units,
};
use crate::guardrail_base::failure::GuardrailFailure;

#[tokio::test]
async fn a_passing_probe_returns_the_parsed_response() {
    let messages = vec![message("user", &json!("hello"))];
    let transport = Scripted::new(vec![Reply::ok(&json!({
        "action": "NONE",
        "usage": { "textUnits": 3 },
    }))]);
    let mut data = BTreeMap::new();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    let response = outcome.expect("the probe passes");
    assert_eq!(response.action.as_deref(), Some("NONE"));
    assert_eq!(transport.count(), 1);
    let records = records_of(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(
        field_of(&records[0], "guardrail_status"),
        expected(&json!("success"))
    );
    assert_eq!(
        field_of(&records[0], "guardrail_provider"),
        expected(&json!("bedrock"))
    );
    assert_eq!(
        field_of(&records[0], "guardrail_name"),
        expected(&json!(GUARDRAIL_NAME))
    );
    assert_eq!(usage_units(&records[0], "textUnits"), expected(&json!(3)));
}

#[tokio::test]
async fn a_non_200_reply_is_an_error_with_the_parsed_message() {
    let messages = vec![message("user", &json!("hello"))];
    let transport = Scripted::new(vec![Reply::failed(
        400,
        &json!({ "message": "invalid content" }),
    )]);
    let mut data = BTreeMap::new();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    let failure = outcome.expect_err("a 400 is a failure");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("a non-200 reply carries a plain http failure");
    };
    assert_eq!(status, 400);
    assert_eq!(detail, "invalid content");
    let records = records_of(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(
        field_of(&records[0], "guardrail_status"),
        expected(&json!("guardrail_failed_to_respond"))
    );
    let response = field_of(&records[0], "guardrail_response");
    assert_eq!(
        field_of(&response, "error"),
        expected(&json!("invalid content"))
    );
    assert!(!has_field(&records[0], "guardrail_usage"));
}

#[tokio::test]
async fn a_non_200_reply_without_a_message_uses_the_default_text() {
    let messages = vec![message("user", &json!("hello"))];
    let transport = Scripted::new(vec![Reply::failed(502, &json!({}))]);
    let mut data = BTreeMap::new();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    let failure = outcome.expect_err("a 502 is a failure");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("a non-200 reply carries a plain http failure");
    };
    assert_eq!(status, 502);
    assert_eq!(detail, "Bedrock guardrail request failed");
}

#[tokio::test]
async fn a_two_hundred_four_reply_is_parsed_as_an_error() {
    let messages = vec![message("user", &json!("hello"))];
    let transport = Scripted::new(vec![Reply::failed(204, &json!(null))]);
    let mut data = BTreeMap::new();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    let failure = outcome.expect_err("a 204 is not the 200 ApplyGuardrail returns");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("a non-200 reply carries a plain http failure");
    };
    assert_eq!(status, 204);
    assert_eq!(detail, "Bedrock guardrail request failed");
    assert_eq!(transport.count(), 1);
}

#[tokio::test]
async fn a_block_is_a_four_hundred_with_the_policy_detail() {
    let messages = vec![message("user", &json!("hello"))];
    let transport = Scripted::new(vec![blocking()]);
    let mut data = BTreeMap::new();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    let failure = outcome.expect_err("a block is a rejection");
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("a block carries the object detail");
    };
    assert_eq!(status, 400);
    let wire = serde_json::to_value(&detail).expect("the detail encodes");
    assert_eq!(wire["error"], json!("Violated guardrail policy"));
    assert_eq!(wire["bedrock_guardrail_response"], json!("blocked text"));
    assert_eq!(wire["guardrailIdentifier"], json!(IDENTIFIER));
    assert_eq!(wire["guardrailVersion"], json!(VERSION));
    let records = records_of(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(
        field_of(&records[0], "guardrail_status"),
        expected(&json!("guardrail_intervened"))
    );
    assert_eq!(usage_units(&records[0], "textUnits"), expected(&json!(7)));
    assert_eq!(transport.count(), 1);
}

#[tokio::test]
async fn a_block_without_the_exception_returns_a_modify_failure() {
    let messages = vec![message("user", &json!("hello"))];
    let transport = Scripted::new(vec![blocking()]);
    let mut data = BTreeMap::new();
    let outcome = run(
        &guardrail_without_block_exception(),
        &mut data,
        input_scan(&messages),
        &transport,
    )
    .await;
    let failure = outcome.expect_err("a block still ends the call");
    let GuardrailFailure::ModifyResponse(modify) = failure else {
        panic!("a disabled block exception is a modify failure");
    };
    assert_eq!(modify.message, "blocked text");
    assert_eq!(modify.guardrail_name.as_deref(), Some(GUARDRAIL_NAME));
    assert_eq!(records_of(&data).len(), 1);
    assert_eq!(transport.count(), 1);
}

#[tokio::test]
async fn an_exception_marker_in_a_two_hundred_body_allows_the_request() {
    let messages = vec![message("user", &json!("hello"))];
    let transport = Scripted::new(vec![Reply::ok(&json!({
        "action": "NONE",
        "Output": { "__type": "ValidationException" },
    }))]);
    let mut data = BTreeMap::new();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    let response = outcome.expect("the marker does not raise");
    assert!(response.extra.contains_key("Output"));
    let records = records_of(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(
        field_of(&records[0], "guardrail_status"),
        expected(&json!("guardrail_failed_to_respond"))
    );
}

#[tokio::test]
async fn a_malformed_two_hundred_body_is_a_server_error_without_a_record() {
    let messages = vec![message("user", &json!("hello"))];
    let transport = Scripted::new(vec![Reply::text(200, "not json")]);
    let mut data = BTreeMap::new();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    let failure = outcome.expect_err("the body does not parse");
    let GuardrailFailure::Http { status, .. } = failure else {
        panic!("an undecodable body is a plain http failure");
    };
    assert_eq!(status, 500);
    assert_eq!(transport.count(), 1);
    assert_eq!(records_of(&data).len(), 0);
}

#[tokio::test]
async fn a_transport_error_is_a_server_error_with_one_record() {
    let messages = vec![message("user", &json!("hello"))];
    let mut data = BTreeMap::new();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &Closed).await;
    let failure = outcome.expect_err("a closed transport fails the call");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("a transport failure is a plain http failure");
    };
    assert_eq!(status, 500);
    assert_eq!(detail, TransportError::Closed.to_string());
    let records = records_of(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(
        field_of(&records[0], "guardrail_status"),
        expected(&json!("guardrail_failed_to_respond"))
    );
    let response = field_of(&records[0], "guardrail_response");
    assert_eq!(
        field_of(&response, "error"),
        expected(&json!(TransportError::Closed.to_string()))
    );
}

#[tokio::test]
async fn a_missing_guardrail_identity_fails_before_the_request() {
    let messages = vec![message("user", &json!("hello"))];
    let transport = Scripted::new(vec![Reply::ok(&json!({ "action": "NONE" }))]);
    let mut data = BTreeMap::new();
    let outcome = run(
        &anonymous_guardrail(),
        &mut data,
        input_scan(&messages),
        &transport,
    )
    .await;
    let failure = outcome.expect_err("the apply path needs an identifier");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("a prepare failure is a plain http failure");
    };
    assert_eq!(status, 500);
    assert!(detail.contains("guardrailIdentifier"));
    assert_eq!(transport.count(), 0);
    assert_eq!(records_of(&data).len(), 0);
}
