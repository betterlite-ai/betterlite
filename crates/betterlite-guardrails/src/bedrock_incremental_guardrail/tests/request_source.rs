#![cfg(test)]

use super::support::{
    Harness, content_texts, expected, field_of, inputs, passing, strings, text, texts_of,
};
use crate::generic_guardrail_contracts::GuardrailInputType;

#[tokio::test]
async fn a_request_scan_sends_input_content() {
    let mut harness = Harness::new(vec![passing()]);

    let result = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Request)
        .await
        .expect("the input scan passes");

    assert_eq!(harness.transport.count(), 1);
    let bodies = harness.transport.bodies();
    assert_eq!(field_of(&bodies[0], "source"), text("INPUT"));
    assert_eq!(
        field_of(&bodies[0], "content"),
        expected(r#"[{"text": {"text": "hello"}}, {"text": {"text": "world"}}]"#)
    );
    assert_eq!(content_texts(&bodies[0]), strings(&["hello", "world"]));
    assert_eq!(texts_of(&result), strings(&["hello", "world"]));
}

#[tokio::test]
async fn an_input_scan_targets_the_apply_guardrail_endpoint() {
    let mut harness = Harness::new(vec![passing()]);

    let _applied = harness
        .apply(inputs(&["hello"]), GuardrailInputType::Request)
        .await
        .expect("the input scan passes");

    let urls = harness.transport.urls();
    assert_eq!(urls.len(), 1);
    assert!(urls[0].contains("gr-abc123"), "{}", urls[0]);
    assert!(urls[0].ends_with("/apply"), "{}", urls[0]);
}
