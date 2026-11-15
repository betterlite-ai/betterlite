#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use rstest::rstest;

use super::process_output_response;
use crate::guardrails::a2a_translation::A2aTranslationError;
use crate::guardrails::a2a_translation::paths::{TextPlacement, extract_texts_from_result};
use crate::guardrails::a2a_translation::tests::support::{
    FakeGuardrail, capture_logs, log_text, value,
};

const EVERY_LOCATION: &str = r#"{"jsonrpc":"2.0","id":1,"result":{
    "parts":[{"kind":"text","text":"direct"}],
    "message":{"parts":[{"kind":"text","text":"nested"}]},
    "artifact":{"parts":[{"kind":"text","text":"artifact"}]},
    "status":{"message":{"parts":[{"kind":"text","text":"status"}]}},
    "artifacts":[{"parts":[{"kind":"text","text":"first"}]},{"parts":[{"kind":"text","text":"second"}]}]}}"#;

fn placement(path: &[&str], part_index: usize, text: &str) -> TextPlacement {
    TextPlacement {
        path: path.iter().map(|key| (*key).to_owned()).collect(),
        part_index,
        text: text.to_owned(),
    }
}

fn result_of(response: &GuardrailValue) -> &BTreeMap<String, GuardrailValue> {
    let GuardrailValue::Object(fields) = response else {
        panic!("response");
    };
    let Some(GuardrailValue::Object(result)) = fields.get("result") else {
        panic!("result");
    };
    result
}

#[tokio::test]
async fn output_collects_every_location_in_one_call_and_writes_the_texts_back() {
    let guardrail = FakeGuardrail::replying(&["a", "b", "c", "d", "e", "f"]);
    let mut response = value(EVERY_LOCATION);
    let original = response.clone();
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    assert_eq!(
        calls[0].inputs.texts,
        Some(vec![
            "direct".to_owned(),
            "nested".to_owned(),
            "artifact".to_owned(),
            "status".to_owned(),
            "first".to_owned(),
            "second".to_owned(),
        ])
    );
    assert_eq!(calls[0].inputs.model, None);
    assert_eq!(calls[0].request_data.get("response"), Some(&original));
    assert_eq!(
        extract_texts_from_result(result_of(&response)),
        vec![
            placement(&["parts"], 0, "a"),
            placement(&["message", "parts"], 0, "b"),
            placement(&["artifact", "parts"], 0, "c"),
            placement(&["status", "message", "parts"], 0, "d"),
            placement(&["artifacts", "0", "parts"], 0, "e"),
            placement(&["artifacts", "1", "parts"], 0, "f"),
        ]
    );
}

#[rstest]
#[case::fewer(&["a"])]
#[case::more(&["a", "b", "c"])]
#[case::empty(&[])]
#[tokio::test]
async fn output_reply_with_a_different_count_leaves_the_response_alone(#[case] reply: &[&str]) {
    let guardrail = FakeGuardrail::replying(reply);
    let mut response =
        value(r#"{"result":{"parts":[{"kind":"text","text":"a"},{"kind":"text","text":"b"}]}}"#);
    let untouched = response.clone();
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(response, untouched);
}

#[tokio::test]
async fn output_dropped_texts_leaves_the_response_alone() {
    let guardrail = FakeGuardrail::dropping_texts();
    let mut response = value(r#"{"result":{"parts":[{"kind":"text","text":"a"}]}}"#);
    let untouched = response.clone();
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(response, untouched);
}

#[rstest]
#[case::no_result(r#"{"jsonrpc":"2.0"}"#)]
#[case::null_result(r#"{"result":null}"#)]
#[case::text_result(r#"{"result":"x"}"#)]
#[case::list_result(r#"{"result":[]}"#)]
#[case::empty_result(r#"{"result":{}}"#)]
#[case::no_text(r#"{"result":{"parts":[{"kind":"data"}]}}"#)]
#[tokio::test]
async fn output_without_a_result_or_text_makes_no_call(#[case] raw: &str) {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut response = value(raw);
    let untouched = response.clone();
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(response, untouched);
    assert!(!log_text(&log).is_empty());
}

#[rstest]
#[case::text("\"a string\"", "str")]
#[case::list("[\"a\"]", "list")]
#[case::integer("7", "int")]
#[case::null("null", "NoneType")]
#[case::boolean("true", "bool")]
#[tokio::test]
async fn output_non_object_response_warns_and_returns_it(#[case] raw: &str, #[case] kind: &str) {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut response = value(raw);
    let untouched = response.clone();
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(response, untouched);
    assert!(
        log_text(&log).contains(&format!(
            "A2A: Unknown response type {kind}, skipping guardrail"
        )),
        "{}",
        log_text(&log)
    );
}

#[tokio::test]
async fn output_keeps_a_response_already_in_the_request_data() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut response = value(r#"{"result":{"parts":[{"kind":"text","text":"a"}]}}"#);
    let mut request_data = BTreeMap::from([(
        "response".to_owned(),
        GuardrailValue::Text("kept".to_owned()),
    )]);
    process_output_response(
        &mut response,
        &guardrail,
        None,
        None,
        Some(&mut request_data),
    )
    .await
    .unwrap();
    assert_eq!(
        request_data.get("response"),
        Some(&GuardrailValue::Text("kept".to_owned()))
    );
    assert_eq!(
        guardrail.calls()[0].request_data.get("response"),
        Some(&GuardrailValue::Text("kept".to_owned()))
    );
}

#[tokio::test]
async fn output_adds_user_metadata_only_when_the_request_data_has_none() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let user = BTreeMap::from([
        ("user_id".to_owned(), GuardrailValue::Text("u-1".to_owned())),
        ("team_id".to_owned(), GuardrailValue::Null),
    ]);
    let mut response = value(r#"{"result":{"parts":[{"kind":"text","text":"a"}]}}"#);
    let mut request_data = BTreeMap::new();
    process_output_response(
        &mut response,
        &guardrail,
        None,
        Some(&user),
        Some(&mut request_data),
    )
    .await
    .unwrap();
    let metadata = BTreeMap::from([(
        "user_api_key_user_id".to_owned(),
        GuardrailValue::Text("u-1".to_owned()),
    )]);
    assert_eq!(
        request_data.get("litellm_metadata"),
        Some(&GuardrailValue::Object(metadata.clone()))
    );
    assert_eq!(
        guardrail.calls()[0].request_data.get("litellm_metadata"),
        Some(&GuardrailValue::Object(metadata))
    );
}

#[tokio::test]
async fn output_keeps_the_litellm_metadata_it_was_given() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let user = BTreeMap::from([("user_id".to_owned(), GuardrailValue::Text("u-1".to_owned()))]);
    let mut response = value(r#"{"result":{"parts":[{"kind":"text","text":"a"}]}}"#);
    let existing = GuardrailValue::Object(BTreeMap::from([(
        "keep".to_owned(),
        GuardrailValue::Boolean(true),
    )]));
    let mut request_data = BTreeMap::from([("litellm_metadata".to_owned(), existing.clone())]);
    process_output_response(
        &mut response,
        &guardrail,
        None,
        Some(&user),
        Some(&mut request_data),
    )
    .await
    .unwrap();
    assert_eq!(request_data.get("litellm_metadata"), Some(&existing));
}

#[rstest]
#[case::no_user(None)]
#[case::empty_user(Some(BTreeMap::new()))]
#[case::null_only(Some(BTreeMap::from([("user_id".to_owned(), GuardrailValue::Null)])))]
#[tokio::test]
async fn output_writes_no_metadata_for_an_empty_user_dict(
    #[case] user: Option<BTreeMap<String, GuardrailValue>>,
) {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut response = value(r#"{"result":{"parts":[{"kind":"text","text":"a"}]}}"#);
    let mut request_data = BTreeMap::new();
    process_output_response(
        &mut response,
        &guardrail,
        None,
        user.as_ref(),
        Some(&mut request_data),
    )
    .await
    .unwrap();
    assert!(!request_data.contains_key("litellm_metadata"));
}

#[tokio::test]
async fn output_guardrail_failure_is_returned_and_leaves_the_response_alone() {
    let failure = GuardrailFailure::Other("blocked by fake".to_owned());
    let guardrail = FakeGuardrail::failing(failure.clone());
    let mut response = value(r#"{"result":{"parts":[{"kind":"text","text":"a"}]}}"#);
    let untouched = response.clone();
    let error = process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap_err();
    match error {
        A2aTranslationError::Guardrail(returned) => assert_eq!(returned, failure),
        A2aTranslationError::Text(_) | A2aTranslationError::Json { .. } => {
            panic!("unexpected error {error:?}")
        }
    }
    assert_eq!(response, untouched);
}
