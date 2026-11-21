#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use tracing::Level;

use super::support::{
    Harness, capture, content_texts, data_messages, inputs, passing, strings, structured_messages,
    texts_of,
};
use crate::generic_guardrail_contracts::GuardrailInputType;

#[tokio::test]
async fn the_latest_user_message_is_the_only_text_scanned() {
    let mut harness = Harness::new(vec![passing()]);
    harness.latest_user_only();
    harness.data.insert(
        "messages".to_owned(),
        data_messages(
            r#"[
                {"role": "user", "content": "first"},
                {"role": "assistant", "content": "reply"},
                {"role": "user", "content": "second"}
            ]"#,
        ),
    );

    let result = harness
        .apply(
            inputs(&["first", "reply", "second"]),
            GuardrailInputType::Request,
        )
        .await
        .expect("the input scan passes");

    assert_eq!(harness.transport.count(), 1);
    assert_eq!(
        content_texts(&harness.transport.bodies()[0]),
        strings(&["second"])
    );
    assert_eq!(texts_of(&result), strings(&["first", "reply", "second"]));
}

#[tokio::test]
async fn structured_messages_take_precedence_over_the_request_messages() {
    let mut harness = Harness::new(vec![passing()]);
    harness.latest_user_only();
    harness.data.insert(
        "messages".to_owned(),
        data_messages(r#"[{"role": "user", "content": "other"}]"#),
    );
    let mut batch_inputs = inputs(&["structured"]);
    batch_inputs.structured_messages = Some(structured_messages(
        r#"[{"role": "user", "content": "structured"}]"#,
    ));

    let _applied = harness
        .apply(batch_inputs, GuardrailInputType::Request)
        .await
        .expect("the input scan passes");

    assert_eq!(
        content_texts(&harness.transport.bodies()[0]),
        strings(&["structured"])
    );
}

#[tokio::test]
async fn without_structured_messages_the_request_messages_select() {
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
        .apply(inputs(&["first", "second"]), GuardrailInputType::Request)
        .await
        .expect("the input scan passes");

    assert_eq!(
        content_texts(&harness.transport.bodies()[0]),
        strings(&["second"])
    );
}

#[tokio::test]
async fn tool_results_only_warns_when_the_filter_leaves_nothing() {
    let (captured, _guard) = capture(Level::DEBUG);
    let mut harness = Harness::new(vec![]);
    harness.latest_user_only();
    harness.tool_results_only = true;
    harness.data.insert(
        "messages".to_owned(),
        data_messages(r#"[{"role": "tool", "content": "result"}]"#),
    );

    let result = harness
        .apply(inputs(&["result"]), GuardrailInputType::Request)
        .await
        .expect("the skip is not a failure");

    assert_eq!(harness.transport.count(), 0);
    assert_eq!(texts_of(&result), strings(&["result"]));
    assert!(
        captured.text().contains(
            "latest-user selection and tool-results-only filtering leave nothing to scan"
        ),
        "{}",
        captured.text()
    );
}

#[tokio::test]
async fn an_empty_batch_never_reaches_bedrock() {
    let mut harness = Harness::new(vec![]);
    harness
        .data
        .insert("api_key".to_owned(), GuardrailValue::UnsignedInteger(1));

    let result = harness
        .apply(inputs(&[]), GuardrailInputType::Request)
        .await
        .expect("an empty batch builds no Bedrock request");

    assert_eq!(harness.transport.count(), 0);
    assert_eq!(result.texts, Some(Vec::new()));
}

#[tokio::test]
async fn an_input_without_texts_scans_nothing() {
    let mut harness = Harness::new(vec![]);

    let result = harness
        .apply(inputs(&[]), GuardrailInputType::Request)
        .await
        .expect("an empty batch is not a failure");

    assert_eq!(harness.transport.count(), 0);
    assert_eq!(result.texts, Some(Vec::new()));
}
