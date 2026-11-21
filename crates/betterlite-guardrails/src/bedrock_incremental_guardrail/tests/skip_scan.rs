#![cfg(test)]

use super::support::{Harness, data_messages, inputs, strings, texts_of};
use crate::generic_guardrail_contracts::GuardrailInputType;

#[tokio::test]
async fn a_request_without_a_user_message_skips_the_scan() {
    let mut harness = Harness::new(vec![]);
    harness.latest_user_only();
    harness.data.insert(
        "messages".to_owned(),
        data_messages(r#"[{"role": "system", "content": "policy"}]"#),
    );

    let result = harness
        .apply(inputs(&["policy"]), GuardrailInputType::Request)
        .await
        .expect("the skip is not a failure");

    assert_eq!(harness.transport.count(), 0);
    assert_eq!(texts_of(&result), strings(&["policy"]));
}

#[tokio::test]
async fn a_latest_user_message_without_text_skips_the_scan() {
    let mut harness = Harness::new(vec![]);
    harness.latest_user_only();
    harness.data.insert(
        "messages".to_owned(),
        data_messages(r#"[{"role": "user"}]"#),
    );

    let result = harness
        .apply(inputs(&["stale"]), GuardrailInputType::Request)
        .await
        .expect("the skip is not a failure");

    assert_eq!(harness.transport.count(), 0);
    assert_eq!(texts_of(&result), strings(&["stale"]));
}

#[tokio::test]
async fn a_skipped_scan_returns_the_inputs_untouched() {
    let mut harness = Harness::new(vec![]);
    harness.latest_user_only();
    harness.data.insert(
        "messages".to_owned(),
        data_messages(r#"[{"role": "system", "content": "policy"}]"#),
    );
    let batch_inputs = inputs(&["policy"]);

    let result = harness
        .apply(batch_inputs.clone(), GuardrailInputType::Request)
        .await
        .expect("the skip is not a failure");

    assert_eq!(result, batch_inputs);
}
