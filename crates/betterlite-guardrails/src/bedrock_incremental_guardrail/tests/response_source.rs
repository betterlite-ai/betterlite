#![cfg(test)]

use super::support::{
    Harness, content_texts, data_messages, expected, field_of, inputs, passing, strings, text,
    texts_of,
};
use crate::generic_guardrail_contracts::GuardrailInputType;

#[tokio::test]
async fn a_response_scan_sends_output_content() {
    let mut harness = Harness::new(vec![passing()]);

    let result = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Response)
        .await
        .expect("the output scan passes");

    assert_eq!(harness.transport.count(), 1);
    let bodies = harness.transport.bodies();
    assert_eq!(field_of(&bodies[0], "source"), text("OUTPUT"));
    assert_eq!(
        field_of(&bodies[0], "content"),
        expected(r#"[{"text": {"text": "hello"}}, {"text": {"text": "world"}}]"#)
    );
    assert_eq!(texts_of(&result), strings(&["hello", "world"]));
}

#[tokio::test]
async fn a_response_scan_keeps_the_legacy_latest_text_selection() {
    let mut harness = Harness::new(vec![passing()]);
    harness.latest_user_only();
    harness.data.insert(
        "messages".to_owned(),
        data_messages(
            r#"[
                {"role": "user", "content": "first"},
                {"role": "user", "content": "second"}
            ]"#,
        ),
    );

    let _applied = harness
        .apply(
            inputs(&["first", "second", "extra"]),
            GuardrailInputType::Response,
        )
        .await
        .expect("the output scan passes");

    assert_eq!(
        content_texts(&harness.transport.bodies()[0]),
        strings(&["extra"])
    );
}

#[tokio::test]
async fn a_response_scan_carries_the_request_messages_for_grounding() {
    let mut harness = Harness::new(vec![passing()]);
    harness.data.insert(
        "messages".to_owned(),
        data_messages(
            r#"[{"role": "system", "content": [{"type": "grounding_source", "text": "policy"}]}]"#,
        ),
    );

    let _applied = harness
        .apply(inputs(&["answer"]), GuardrailInputType::Response)
        .await
        .expect("the output scan passes");

    let bodies = harness.transport.bodies();
    assert_eq!(
        field_of(&bodies[0], "content"),
        expected(
            r#"[
                {"text": {"text": "policy", "qualifiers": ["grounding_source"]}},
                {"text": {"text": "answer", "qualifiers": ["guard_content"]}}
            ]"#
        )
    );
    assert_eq!(content_texts(&bodies[0]), strings(&["policy", "answer"]));
}
