#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use rstest::rstest;

use super::process_input_messages;
use crate::guardrails::a2a_translation::A2aTranslationError;
use crate::guardrails::a2a_translation::tests::support::{
    FakeGuardrail, body, capture_logs, log_text,
};

const MESSAGE: &str = r#"{"model":"agent-model","params":{"message":{"kind":"message","messageId":"m-1","role":"user","parts":[{"kind":"text","text":"hello"},{"kind":"data","data":{}},{"kind":"text","text":"world"}]}}}"#;

fn part_text(data: &BTreeMap<String, GuardrailValue>, index: usize) -> &GuardrailValue {
    let GuardrailValue::Object(params) = data.get("params").unwrap() else {
        panic!("params");
    };
    let GuardrailValue::Object(message) = params.get("message").unwrap() else {
        panic!("message");
    };
    let GuardrailValue::Array(parts) = message.get("parts").unwrap() else {
        panic!("parts");
    };
    let Some(GuardrailValue::Object(part)) = parts.get(index) else {
        panic!("part");
    };
    part.get("text").unwrap()
}

#[tokio::test]
async fn input_sends_the_text_parts_with_the_structured_message_and_model() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["first", "second"]);
    let mut data = body(MESSAGE);
    let original = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(
        part_text(&data, 0),
        &GuardrailValue::Text("first".to_owned())
    );
    assert_eq!(
        part_text(&data, 2),
        &GuardrailValue::Text("second".to_owned())
    );
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Request);
    assert_eq!(
        calls[0].inputs.texts,
        Some(vec!["hello".to_owned(), "world".to_owned()])
    );
    assert_eq!(calls[0].inputs.model, Some("agent-model".to_owned()));
    assert_eq!(calls[0].request_data, original);
    let structured = calls[0].inputs.structured_messages.as_ref().unwrap();
    assert_eq!(structured.len(), 1);
    assert_eq!(structured[0].role, "user");
    assert!(!structured[0].extra.contains_key("role"));
    assert_eq!(
        structured[0].extra.get("messageId"),
        Some(&GuardrailValue::Text("m-1".to_owned()))
    );
    assert!(structured[0].extra.contains_key("parts"));
    assert!(
        log_text(&log).contains("A2A: Processed input message with 2 text parts"),
        "{}",
        log_text(&log)
    );
}

#[tokio::test]
async fn input_skips_empty_text_and_non_text_parts() {
    let guardrail = FakeGuardrail::replying(&["x", "y"]);
    let mut data = body(
        r#"{"params":{"message":{"role":"user","parts":[{"kind":"text","text":""},{"kind":"data"},{"kind":"text","text":"a"},{"text":"no kind"},{"kind":"text","text":7},{"kind":"text","text":"b"}]}}}"#,
    );
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(
        guardrail.calls()[0].inputs.texts,
        Some(vec!["a".to_owned(), "b".to_owned()])
    );
    assert_eq!(
        data,
        body(
            r#"{"params":{"message":{"role":"user","parts":[{"kind":"text","text":""},{"kind":"data"},{"kind":"text","text":"x"},{"text":"no kind"},{"kind":"text","text":7},{"kind":"text","text":"y"}]}}}"#
        )
    );
}

#[rstest]
#[case::fewer(&["one"], false)]
#[case::more(&["one", "two", "three"], false)]
#[case::empty_reply(&[], false)]
#[case::dropped(&[], true)]
#[tokio::test]
async fn input_reply_with_a_different_count_leaves_every_part(
    #[case] reply: &[&str],
    #[case] drops: bool,
) {
    let guardrail = if drops {
        FakeGuardrail::dropping_texts()
    } else {
        FakeGuardrail::replying(reply)
    };
    let mut data = body(MESSAGE);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(data, untouched);
}

#[tokio::test]
async fn input_passing_guardrail_keeps_the_texts_it_sent() {
    let guardrail = FakeGuardrail::passing_through();
    let mut data = body(MESSAGE);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(data, untouched);
    assert_eq!(guardrail.calls().len(), 1);
}

#[tokio::test]
async fn input_one_empty_text_clears_the_part() {
    let guardrail = FakeGuardrail::replying(&[""]);
    let mut data =
        body(r#"{"params":{"message":{"role":"user","parts":[{"kind":"text","text":"hello"}]}}}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(part_text(&data, 0), &GuardrailValue::Text(String::new()));
}

#[rstest]
#[case::no_params(r#"{"model":"m"}"#)]
#[case::params_not_an_object(r#"{"params":"x"}"#)]
#[case::no_message(r#"{"params":{}}"#)]
#[case::message_not_an_object(r#"{"params":{"message":7}}"#)]
#[case::no_parts(r#"{"params":{"message":{"role":"user"}}}"#)]
#[case::parts_not_an_array(r#"{"params":{"message":{"role":"user","parts":null}}}"#)]
#[case::parts_empty(r#"{"params":{"message":{"role":"user","parts":[]}}}"#)]
#[tokio::test]
async fn input_without_parts_makes_no_call_and_logs_at_debug(#[case] raw: &str) {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut data = body(raw);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(data, untouched);
    assert!(
        log_text(&log).contains("A2A: No parts in message, skipping guardrail"),
        "{}",
        log_text(&log)
    );
}

#[tokio::test]
async fn input_without_text_parts_makes_no_call() {
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut data = body(r#"{"params":{"message":{"role":"user","parts":[{"kind":"data"}]}}}"#);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(data, untouched);
}

#[rstest]
#[case::text(
    r#"{"model":"m","params":{"message":{"role":"user","parts":[{"kind":"text","text":"hi"}]}}}"#,
    Some("m")
)]
#[case::empty(
    r#"{"model":"","params":{"message":{"role":"user","parts":[{"kind":"text","text":"hi"}]}}}"#,
    None
)]
#[case::integer(
    r#"{"model":7,"params":{"message":{"role":"user","parts":[{"kind":"text","text":"hi"}]}}}"#,
    None
)]
#[case::null(
    r#"{"model":null,"params":{"message":{"role":"user","parts":[{"kind":"text","text":"hi"}]}}}"#,
    None
)]
#[case::absent(
    r#"{"params":{"message":{"role":"user","parts":[{"kind":"text","text":"hi"}]}}}"#,
    None
)]
#[tokio::test]
async fn input_model_is_sent_only_when_it_is_a_non_empty_text(
    #[case] raw: &str,
    #[case] expected: Option<&str>,
) {
    let guardrail = FakeGuardrail::replying(&["hi"]);
    let mut data = body(raw);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(guardrail.calls()[0].inputs.model.as_deref(), expected);
}

#[rstest]
#[case::no_role(r#"{"kind":"message","parts":[{"kind":"text","text":"hi"}]}"#)]
#[case::null_role(r#"{"role":null,"parts":[{"kind":"text","text":"hi"}]}"#)]
#[case::integer_role(r#"{"role":7,"parts":[{"kind":"text","text":"hi"}]}"#)]
#[tokio::test]
async fn input_message_without_a_text_role_is_scanned_with_an_empty_role(#[case] message: &str) {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let raw = format!(r#"{{"params":{{"message":{message}}}}}"#);
    let mut data = body(&raw);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls[0].inputs.texts, Some(vec!["hi".to_owned()]));
    let structured = calls[0].inputs.structured_messages.as_ref().unwrap();
    assert_eq!(structured[0].role, String::new());
    assert!(!structured[0].extra.contains_key("role"));
    assert_eq!(
        part_text(&data, 0),
        &GuardrailValue::Text("rewritten".to_owned())
    );
}

#[tokio::test]
async fn input_guardrail_failure_is_returned_and_leaves_the_data_alone() {
    let failure = GuardrailFailure::Other("blocked by fake".to_owned());
    let guardrail = FakeGuardrail::failing(failure.clone());
    let mut data = body(MESSAGE);
    let untouched = data.clone();
    let error = process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap_err();
    match error {
        A2aTranslationError::Guardrail(returned) => assert_eq!(returned, failure),
        A2aTranslationError::Text(_) | A2aTranslationError::Json { .. } => {
            panic!("unexpected error {error:?}")
        }
    }
    assert_eq!(data, untouched);
}

#[tokio::test]
async fn input_parts_that_are_not_objects_keep_their_index() {
    let guardrail = FakeGuardrail::replying(&["changed"]);
    let mut data = body(
        r#"{"params":{"message":{"role":"user","parts":[7,[],"x",{"kind":"text","text":"found"}]}}}"#,
    );
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(
        guardrail.calls()[0].inputs.texts,
        Some(vec!["found".to_owned()])
    );
    assert_eq!(
        data,
        body(
            r#"{"params":{"message":{"role":"user","parts":[7,[],"x",{"kind":"text","text":"changed"}]}}}"#
        )
    );
}
