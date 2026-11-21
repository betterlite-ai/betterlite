#![cfg(test)]

use tracing::Level;

use super::support::{Harness, SESSION, capture, inputs, passing};
use crate::generic_guardrail_contracts::GuardrailInputType;

#[tokio::test]
async fn a_session_with_nothing_new_logs_the_skip() {
    let (captured, _guard) = capture(Level::DEBUG);
    let mut harness = Harness::new(vec![]).with_session();
    harness.incremental();
    harness.cache.remember(SESSION, &["hello"]);

    let _applied = harness
        .apply(inputs(&["hello"]), GuardrailInputType::Request)
        .await
        .expect("the skip is not a failure");

    assert!(
        captured.text().contains(
            "Bedrock Guardrail: no new messages to scan for this session, skipping API call"
        ),
        "{}",
        captured.text()
    );
}

#[tokio::test]
async fn an_empty_batch_logs_zero_texts() {
    let (captured, _guard) = capture(Level::DEBUG);
    let mut harness = Harness::new(vec![passing()]);

    let _applied = harness
        .apply(inputs(&[]), GuardrailInputType::Request)
        .await
        .expect("an empty batch is not a failure");

    assert!(
        captured
            .text()
            .contains("Bedrock Guardrail: Applying guardrail to 0 text(s)"),
        "{}",
        captured.text()
    );
}
