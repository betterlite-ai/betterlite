#![cfg(test)]

use super::support::{Harness, field_of, inputs, passing, records_of, text};
use crate::generic_guardrail_contracts::GuardrailInputType;

#[tokio::test]
async fn a_request_scan_runs_as_a_pre_call_hook() {
    let mut harness = Harness::new(vec![passing()]);

    let _applied = harness
        .apply(inputs(&["hello"]), GuardrailInputType::Request)
        .await
        .expect("the input scan passes");

    let records = records_of(&harness.data);
    assert_eq!(records.len(), 1);
    assert_eq!(field_of(&records[0], "guardrail_mode"), text("pre_call"));
}

#[tokio::test]
async fn a_response_scan_runs_as_a_post_call_hook() {
    let mut harness = Harness::new(vec![passing()]);

    let _applied = harness
        .apply(inputs(&["hello"]), GuardrailInputType::Response)
        .await
        .expect("the output scan passes");

    let records = records_of(&harness.data);
    assert_eq!(records.len(), 1);
    assert_eq!(field_of(&records[0], "guardrail_mode"), text("post_call"));
}
