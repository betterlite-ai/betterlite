#![cfg(test)]

use tracing::Level;

use super::batch::wrap_unexpected;
use super::support::{Harness, SESSION, capture, data_messages, inputs, masked, passing};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;

#[tokio::test]
async fn the_entry_log_counts_the_texts() {
    let (captured, _guard) = capture(Level::DEBUG);
    let mut harness = Harness::new(vec![passing()]);

    let _applied = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Request)
        .await
        .expect("the input scan passes");

    assert!(
        captured
            .text()
            .contains("Bedrock Guardrail: Applying guardrail to 2 text(s)"),
        "{}",
        captured.text()
    );
}

#[tokio::test]
async fn a_completed_scan_logs_success() {
    let (captured, _guard) = capture(Level::DEBUG);
    let mut harness = Harness::new(vec![passing()]);

    let _applied = harness
        .apply(inputs(&["hello"]), GuardrailInputType::Request)
        .await
        .expect("the input scan passes");

    assert!(
        captured
            .text()
            .contains("Bedrock Guardrail: Successfully applied guardrail"),
        "{}",
        captured.text()
    );
}

#[tokio::test]
async fn a_skipped_scan_logs_no_success() {
    let (captured, _guard) = capture(Level::DEBUG);
    let mut harness = Harness::new(vec![]);
    harness.latest_user_only();
    harness.data.insert(
        "messages".to_owned(),
        data_messages(r#"[{"role": "system", "content": "policy"}]"#),
    );

    let _applied = harness
        .apply(inputs(&["policy"]), GuardrailInputType::Request)
        .await
        .expect("the skip is not a failure");

    let logs = captured.text();
    assert!(logs.contains("Bedrock Guardrail: Applying guardrail to 1 text(s)"));
    assert!(!logs.contains("Successfully applied guardrail"), "{logs}");
}

#[tokio::test]
async fn a_masked_incremental_scan_warns_with_the_guardrail_name() {
    let (captured, _guard) = capture(Level::DEBUG);
    let mut harness = Harness::new(vec![masked("MASKED"), passing()]).with_session();
    harness.incremental();
    harness.cache.remember(SESSION, &["hello"]);

    let _applied = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Request)
        .await
        .expect("the fallback full scan passes");

    assert!(
        captured.text().contains(
            "Bedrock Guardrail bedrock-incremental: guardrail returned masked/anonymized content"
        ),
        "{}",
        captured.text()
    );
}

#[test]
fn a_wrapped_failure_logs_the_error() {
    let (captured, _guard) = capture(Level::ERROR);

    let wrapped = wrap_unexpected(GuardrailFailure::Other("boom".to_owned()));

    assert_eq!(
        wrapped,
        GuardrailFailure::Other("Bedrock guardrail failed: boom".to_owned())
    );
    let logs = captured.text();
    assert!(logs.contains("ERROR"), "{logs}");
    assert!(
        logs.contains("Bedrock Guardrail: Failed to apply guardrail: boom"),
        "{logs}"
    );
}
