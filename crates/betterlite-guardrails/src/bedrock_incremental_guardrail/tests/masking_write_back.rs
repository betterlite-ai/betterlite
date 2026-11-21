#![cfg(test)]

use super::support::{Harness, Reply, data_messages, inputs, masked, passing, strings, texts_of};
use crate::generic_guardrail_contracts::GuardrailInputType;

#[tokio::test]
async fn a_masked_response_replaces_the_whole_list() {
    let mut harness = Harness::new(vec![Reply::ok(
        r#"{"action": "NONE", "outputs": [{"text": "MASK A"}, {"text": "MASK B"}]}"#,
    )]);

    let result = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Request)
        .await
        .expect("the input scan passes");

    assert_eq!(texts_of(&result), strings(&["MASK A", "MASK B"]));
}

#[tokio::test]
async fn a_masked_subset_writes_back_to_the_latest_user_message() {
    let mut harness = Harness::new(vec![masked("MASKED")]);
    harness.latest_user_only();
    harness.data.insert(
        "messages".to_owned(),
        data_messages(
            r#"[
                {"role": "user", "content": "hello"},
                {"role": "user", "content": "world"}
            ]"#,
        ),
    );

    let result = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Request)
        .await
        .expect("the input scan passes");

    assert_eq!(texts_of(&result), strings(&["hello", "MASKED"]));
}

#[tokio::test]
async fn the_merged_response_writes_back_the_outputs_list() {
    let mut harness = Harness::new(vec![Reply::ok(
        r#"{"action": "NONE", "output": [{"text": "A"}, {"text": "B"}], "outputs": [{"text": "X"}, {"text": "Y"}]}"#,
    )]);

    let result = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Request)
        .await
        .expect("the input scan passes");

    assert_eq!(texts_of(&result), strings(&["X", "Y"]));
}

#[tokio::test]
async fn an_unaligned_subset_keeps_the_originals() {
    let mut harness = Harness::new(vec![masked("MASKED")]);
    harness.latest_user_only();
    harness.data.insert(
        "messages".to_owned(),
        data_messages(r#"[{"role": "user", "content": "a"}]"#),
    );

    let result = harness
        .apply(inputs(&["a", "b"]), GuardrailInputType::Request)
        .await
        .expect("the input scan passes");

    assert_eq!(texts_of(&result), strings(&["a", "b"]));
}

#[tokio::test]
async fn an_unmasked_scan_leaves_the_texts_alone() {
    let mut harness = Harness::new(vec![passing()]);

    let result = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Request)
        .await
        .expect("the input scan passes");

    assert_eq!(texts_of(&result), strings(&["hello", "world"]));
}
