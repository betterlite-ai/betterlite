use std::collections::BTreeMap;

use serde_json::json;

use super::{
    Reply, Scripted, blocking, guardrail_with_budget, input_scan, message, passing, record_field,
    records_of, run, too_large, usage_units,
};
use crate::bedrock_message_selection::wire::Message;
use crate::guardrail_base::failure::GuardrailFailure;

fn two_items() -> Vec<Message> {
    vec![
        message("user", json!("aaaa")),
        message("user", json!("bbbb")),
    ]
}

#[tokio::test]
async fn a_successful_call_writes_one_record_with_the_merged_usage() {
    let messages = two_items();
    let transport = Scripted::new(vec![too_large(), passing(5), passing(7)]);
    let mut data = BTreeMap::new();
    let outcome = run(
        &guardrail_with_budget(5),
        &mut data,
        input_scan(&messages),
        &transport,
    )
    .await;
    drop(outcome.expect("both batches pass"));
    assert_eq!(transport.count(), 3);
    let records = records_of(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(
        record_field(&records[0], "guardrail_status"),
        json!("success")
    );
    assert_eq!(usage_units(&records[0], "textUnits"), json!(12));
}

#[tokio::test]
async fn a_failure_after_a_chunk_logs_once_with_the_billed_usage() {
    let messages = two_items();
    let transport = Scripted::new(vec![
        too_large(),
        passing(5),
        Reply::failed(400, json!({ "message": "invalid content" })),
    ]);
    let mut data = BTreeMap::new();
    let outcome = run(
        &guardrail_with_budget(5),
        &mut data,
        input_scan(&messages),
        &transport,
    )
    .await;
    let failure = outcome.expect_err("the second batch fails");
    let GuardrailFailure::Http { status, .. } = failure else {
        panic!("a 400 is a plain http failure");
    };
    assert_eq!(status, 400);
    assert_eq!(transport.count(), 3);
    let records = records_of(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(
        record_field(&records[0], "guardrail_status"),
        json!("guardrail_failed_to_respond")
    );
    let response = record_field(&records[0], "guardrail_response");
    assert_eq!(response["error"], json!("invalid content"));
    assert_eq!(usage_units(&records[0], "textUnits"), json!(5));
}

#[tokio::test]
async fn a_block_after_a_chunk_logs_the_attempt_with_the_billed_usage() {
    let messages = two_items();
    let transport = Scripted::new(vec![too_large(), passing(5), blocking()]);
    let mut data = BTreeMap::new();
    let outcome = run(
        &guardrail_with_budget(5),
        &mut data,
        input_scan(&messages),
        &transport,
    )
    .await;
    let failure = outcome.expect_err("the block ends the call");
    let GuardrailFailure::HttpObject { status, .. } = failure else {
        panic!("a block carries the object detail");
    };
    assert_eq!(status, 400);
    assert_eq!(transport.count(), 3);
    let records = records_of(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(
        record_field(&records[0], "guardrail_status"),
        json!("guardrail_intervened")
    );
    assert_eq!(usage_units(&records[0], "textUnits"), json!(12));
}
