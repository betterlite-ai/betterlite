use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::{
    Reply, Scripted, expected, field_of, guardrail, input_scan, message, model_response,
    output_scan, records_of, run, text,
};
use crate::guardrail_base::failure::GuardrailFailure;

#[tokio::test]
async fn an_empty_content_list_skips_the_request() {
    let transport = Scripted::new(vec![Reply::ok(r#"{ "action": "NONE" }"#)]);
    let mut data = BTreeMap::new();
    let outcome = run(&guardrail(), &mut data, input_scan(&[]), &transport).await;
    let response = outcome.expect("an empty turn is skipped");
    assert_eq!(response.action, None);
    assert_eq!(transport.count(), 0);
    assert_eq!(records_of(&data).len(), 0);
}

#[tokio::test]
async fn dynamic_params_land_in_the_body_without_content_or_source() {
    let wire = r#"{
        "api_key": "sk-test",
        "metadata": {
            "guardrails": [{
                "bedrock-apply": {
                    "extra_body": { "some_flag": true, "content": "SENTINEL", "source": "SENTINEL" }
                }
            }]
        }
    }"#;
    let mut data: BTreeMap<String, GuardrailValue> =
        serde_json::from_str(wire).expect("the request data parses");
    let messages = vec![message("user", "hello")];
    let transport = Scripted::new(vec![Reply::ok(r#"{ "action": "NONE" }"#)]);
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    drop(outcome.expect("the probe passes"));
    let bodies = transport.bodies();
    assert_eq!(field_of(&bodies[0], "some_flag"), expected("true"));
    assert_eq!(
        field_of(&bodies[0], "content"),
        expected(r#"[{ "text": { "text": "hello" } }]"#)
    );
    assert_eq!(field_of(&bodies[0], "source"), text("INPUT"));
    assert_eq!(
        transport.authorizations(),
        vec![Some("Bearer sk-test".to_owned())]
    );
}

#[tokio::test]
async fn an_input_scan_defaults_the_event_type_to_pre_call() {
    let messages = vec![message("user", "hello")];
    let transport = Scripted::new(vec![Reply::ok(r#"{ "action": "NONE" }"#)]);
    let mut data = BTreeMap::new();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    drop(outcome.expect("the probe passes"));
    let records = records_of(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(field_of(&records[0], "guardrail_mode"), text("pre_call"));
}

#[tokio::test]
async fn an_output_scan_defaults_the_event_type_to_post_call() {
    let messages = vec![message("user", "hello")];
    let response = model_response("the answer");
    let transport = Scripted::new(vec![Reply::ok(r#"{ "action": "NONE" }"#)]);
    let mut data = BTreeMap::new();
    let outcome = run(
        &guardrail(),
        &mut data,
        output_scan(Some(&messages), &response),
        &transport,
    )
    .await;
    drop(outcome.expect("the probe passes"));
    let records = records_of(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(field_of(&records[0], "guardrail_mode"), text("post_call"));
}

#[tokio::test]
async fn an_explicit_event_type_wins_over_the_source() {
    let messages = vec![message("user", "hello")];
    let mut scan = input_scan(&messages);
    scan.logging_event_type = Some(GuardrailEventHooks::DuringCall);
    let transport = Scripted::new(vec![Reply::ok(r#"{ "action": "NONE" }"#)]);
    let mut data = BTreeMap::new();
    let outcome = run(&guardrail(), &mut data, scan, &transport).await;
    drop(outcome.expect("the probe passes"));
    let records = records_of(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(field_of(&records[0], "guardrail_mode"), text("during_call"));
}

#[tokio::test]
async fn a_non_string_api_key_is_rejected_by_name() {
    let mut data: BTreeMap<String, GuardrailValue> =
        serde_json::from_str(r#"{ "api_key": { "value": "sk-test" } }"#)
            .expect("the request data parses");
    let messages = vec![message("user", "hello")];
    let transport = Scripted::new(vec![Reply::ok(r#"{ "action": "NONE" }"#)]);
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    let failure = outcome.expect_err("a non-string api key is rejected");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("the api key check reports a plain http failure");
    };
    assert_eq!(status, 500);
    assert!(detail.contains("api_key"));
    assert_eq!(transport.count(), 0);
}
