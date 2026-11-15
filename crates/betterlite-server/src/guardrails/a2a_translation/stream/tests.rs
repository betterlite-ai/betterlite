#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use rstest::rstest;

use super::{get_streaming_scan_key, process_streaming_response};
use crate::guardrails::a2a_translation::A2aTranslationError;
use crate::guardrails::a2a_translation::tests::support::{
    FakeGuardrail, capture_logs, log_text, value,
};

fn chunks(raw: &[&str]) -> Vec<GuardrailValue> {
    raw.iter().map(|item| value(item)).collect()
}

fn text_of(item: &GuardrailValue) -> String {
    match item {
        GuardrailValue::Object(fields) => {
            let Some(GuardrailValue::Object(result)) = fields.get("result") else {
                panic!("result");
            };
            let Some(GuardrailValue::Object(artifact)) = result.get("artifact") else {
                panic!("artifact");
            };
            let Some(GuardrailValue::Array(parts)) = artifact.get("parts") else {
                panic!("parts");
            };
            let Some(GuardrailValue::Object(part)) = parts.first() else {
                panic!("part");
            };
            let Some(GuardrailValue::Text(text)) = part.get("text") else {
                panic!("text");
            };
            text.clone()
        }
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_) => panic!("unexpected chunk {item:?}"),
    }
}

const FIRST: &str =
    r#"{"jsonrpc":"2.0","id":1,"result":{"artifact":{"parts":[{"kind":"text","text":"he"}]}}}"#;
const SECOND: &str =
    r#"{"jsonrpc":"2.0","id":2,"result":{"artifact":{"parts":[{"kind":"text","text":"llo"}]}}}"#;

#[tokio::test]
async fn stream_joins_the_item_texts_and_scans_once() {
    let guardrail = FakeGuardrail::replying(&["REDACTED"]);
    let mut responses = chunks(&[FIRST, SECOND]);
    let original = responses.clone();
    process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    assert_eq!(calls[0].inputs.texts, Some(vec!["hello".to_owned()]));
    assert_eq!(
        calls[0].request_data.get("responses_so_far"),
        Some(&GuardrailValue::Array(original))
    );
    assert_eq!(text_of(&responses[0]), "REDACTED");
    assert_eq!(text_of(&responses[1]), String::new());
}

#[tokio::test]
async fn stream_keeps_only_the_first_text_part_of_the_first_chunk() {
    let guardrail = FakeGuardrail::replying(&["all"]);
    let mut responses = chunks(&[
        r#"{"result":{"parts":[{"kind":"text","text":"a"},{"kind":"text","text":"b"}]}}"#,
        r#"{"result":{"status":{"message":{"parts":[{"kind":"text","text":"c"}]}}}}"#,
    ]);
    process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap();
    let GuardrailValue::Object(fields) = &responses[0] else {
        panic!("chunk");
    };
    let Some(GuardrailValue::Object(result)) = fields.get("result") else {
        panic!("result");
    };
    let Some(GuardrailValue::Array(parts)) = result.get("parts") else {
        panic!("parts");
    };
    assert_eq!(
        parts.first(),
        Some(&value(r#"{"kind":"text","text":"all"}"#))
    );
    assert_eq!(parts.get(1), Some(&value(r#"{"kind":"text","text":""}"#)));
}

#[tokio::test]
async fn stream_rewrites_an_ndjson_item_as_a_compact_line() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let raw = concat!(
        r#"{"jsonrpc":"2.0","id":1,"result":{"parts":[{"kind":"text","text":"hi"}]}}"#,
        "\n"
    );
    let mut responses = vec![GuardrailValue::Text(raw.to_owned())];
    process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        guardrail.calls()[0].inputs.texts,
        Some(vec!["hi".to_owned()])
    );
    let expected = value(
        r#"{"jsonrpc":"2.0","id":1,"result":{"parts":[{"kind":"text","text":"rewritten"}]}}"#,
    );
    let line = format!("{}\n", serde_json::to_string(&expected).unwrap());
    assert_eq!(responses, vec![GuardrailValue::Text(line)]);
}

#[tokio::test]
async fn stream_rewrites_a_parsed_ndjson_item_that_has_no_text() {
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut responses = vec![
        GuardrailValue::Text(
            r#"{"jsonrpc":"2.0","id":1,"result":{"artifact":{"parts":[{"kind":"text","text":"hi"}]}}}"#
                .to_owned(),
        ),
        GuardrailValue::Text(r#"{"jsonrpc":"2.0","id":2,"result":{"kind":"task"}}"#.to_owned()),
    ];
    process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap();
    let expected = value(r#"{"jsonrpc":"2.0","id":2,"result":{"kind":"task"}}"#);
    assert_eq!(
        responses.get(1),
        Some(&GuardrailValue::Text(format!(
            "{}\n",
            serde_json::to_string(&expected).unwrap()
        )))
    );
}

#[tokio::test]
async fn stream_leaves_invalid_items_alone() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut responses = vec![
        GuardrailValue::Text("not json".to_owned()),
        value("7"),
        value(r#"{"result":"not an object"}"#),
        value(r#"{"result":{"parts":[{"kind":"text","text":"hi"}]}}"#),
    ];
    let before_first = responses[0].clone();
    let before_second = responses[1].clone();
    let before_third = responses[2].clone();
    process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(responses[0], before_first);
    assert_eq!(responses[1], before_second);
    assert_eq!(responses[2], before_third);
    assert_eq!(
        guardrail.calls()[0].inputs.texts,
        Some(vec!["hi".to_owned()])
    );
}

#[rstest]
#[case::no_items(&[])]
#[case::only_invalid(&["7", "\"not json\""])]
#[case::no_text(&[r#"{"result":{"kind":"task"}}"#])]
#[tokio::test]
async fn stream_without_a_valid_text_makes_no_call(#[case] raw: &[&str]) {
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut responses = chunks(raw);
    let untouched = responses.clone();
    process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(responses, untouched);
}

#[rstest]
#[case::fewer(&["one"])]
#[case::more(&["one", "two"])]
#[tokio::test]
async fn stream_uses_only_the_first_returned_text(#[case] reply: &[&str]) {
    let guardrail = FakeGuardrail::replying(reply);
    let mut responses = chunks(&[FIRST, SECOND]);
    process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(text_of(&responses[0]), "one");
    assert_eq!(text_of(&responses[1]), String::new());
}

#[tokio::test]
async fn stream_empty_reply_leaves_the_chunks_alone() {
    let guardrail = FakeGuardrail::replying(&[]);
    let mut responses = chunks(&[FIRST, SECOND]);
    let untouched = responses.clone();
    process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(responses, untouched);
}

#[tokio::test]
async fn stream_dropped_texts_leaves_the_chunks_alone() {
    let guardrail = FakeGuardrail::dropping_texts();
    let mut responses = chunks(&[FIRST]);
    let untouched = responses.clone();
    process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(responses, untouched);
}

#[tokio::test]
async fn stream_reads_only_the_first_location_of_each_item() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut responses = chunks(&[
        r#"{"result":{"parts":[{"kind":"text","text":"direct"}],"message":{"parts":[{"kind":"text","text":"nested"}]}}}"#,
        r#"{"result":{"artifacts":[{"parts":[{"kind":"text","text":"first"}]},{"parts":[{"kind":"text","text":"second"}]}]}}"#,
    ]);
    process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        guardrail.calls()[0].inputs.texts,
        Some(vec!["directfirst".to_owned()])
    );
}

#[tokio::test]
async fn stream_keeps_the_litellm_metadata_it_was_given() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let user = BTreeMap::from([("user_id".to_owned(), GuardrailValue::Text("u-1".to_owned()))]);
    let mut responses = chunks(&[FIRST]);
    let mut request_data = BTreeMap::new();
    process_streaming_response(
        &mut responses,
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
        Some(&GuardrailValue::Object(metadata))
    );
}

#[tokio::test]
async fn stream_keeps_the_metadata_and_responses_it_was_given() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let user = BTreeMap::from([("user_id".to_owned(), GuardrailValue::Text("u-1".to_owned()))]);
    let mut responses = chunks(&[FIRST]);
    let given_metadata = GuardrailValue::Object(BTreeMap::from([(
        "keep".to_owned(),
        GuardrailValue::Boolean(true),
    )]));
    let given_responses = GuardrailValue::Text("kept".to_owned());
    let mut request_data = BTreeMap::from([
        ("litellm_metadata".to_owned(), given_metadata.clone()),
        ("responses_so_far".to_owned(), given_responses.clone()),
    ]);
    process_streaming_response(
        &mut responses,
        &guardrail,
        None,
        Some(&user),
        Some(&mut request_data),
    )
    .await
    .unwrap();
    assert_eq!(request_data.get("litellm_metadata"), Some(&given_metadata));
    assert_eq!(request_data.get("responses_so_far"), Some(&given_responses));
    let recorded = &guardrail.calls()[0].request_data;
    assert_eq!(recorded.get("litellm_metadata"), Some(&given_metadata));
    assert_eq!(recorded.get("responses_so_far"), Some(&given_responses));
}

#[tokio::test]
async fn stream_writes_the_text_into_the_first_chunk_that_has_text() {
    let guardrail = FakeGuardrail::replying(&["X"]);
    let mut responses = chunks(&[
        r#"{"result":{"kind":"task","status":{"state":"submitted"}}}"#,
        r#"{"result":{"artifact":{"parts":[{"kind":"text","text":"he"}]}}}"#,
        r#"{"result":{"artifact":{"parts":[{"kind":"text","text":"llo"}]}}}"#,
    ]);
    let opening = responses[0].clone();
    process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        guardrail.calls()[0].inputs.texts,
        Some(vec!["hello".to_owned()])
    );
    assert_eq!(responses[0], opening);
    assert_eq!(text_of(&responses[1]), "X");
    assert_eq!(text_of(&responses[2]), String::new());
}

#[tokio::test]
async fn stream_guardrail_failure_is_returned_and_leaves_the_chunks_alone() {
    let failure = GuardrailFailure::Other("blocked by fake".to_owned());
    let guardrail = FakeGuardrail::failing(failure.clone());
    let mut responses = chunks(&[FIRST, SECOND]);
    let untouched = responses.clone();
    let error = process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap_err();
    match error {
        A2aTranslationError::Guardrail(returned) => assert_eq!(returned, failure),
        A2aTranslationError::Text(_) | A2aTranslationError::Json { .. } => {
            panic!("unexpected error {error:?}")
        }
    }
    assert_eq!(responses, untouched);
}

#[tokio::test]
async fn stream_unreadable_parts_are_an_error() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut responses = chunks(&[r#"{"result":{"parts":7}}"#]);
    let untouched = responses.clone();
    let error = process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap_err();
    assert!(matches!(error, A2aTranslationError::Text(_)));
    assert_eq!(responses, untouched);
}

#[test]
fn scan_key_holds_the_joined_text() {
    let key = get_streaming_scan_key(&chunks(&[FIRST, SECOND]));
    assert_eq!(key.texts, vec!["hello".to_owned()]);
}

#[rstest]
#[case::no_items(&[])]
#[case::no_text(&[r#"{"result":{"kind":"task"}}"#])]
#[test]
fn scan_key_holds_an_empty_text_when_there_is_nothing_to_scan(#[case] raw: &[&str]) {
    let key = get_streaming_scan_key(&chunks(raw));
    assert_eq!(key.texts, vec![String::new()]);
    assert!(key.has_nothing_to_scan());
}

#[test]
fn scan_key_skips_an_unreadable_chunk() {
    let key = get_streaming_scan_key(&chunks(&[
        r#"{"result":{"parts":7}}"#,
        r#"{"result":{"parts":[{"kind":"text","text":"ok"}]}}"#,
    ]));
    assert_eq!(key.texts, vec!["ok".to_owned()]);
}

#[tokio::test]
async fn stream_logs_nothing_at_warn_level() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut responses = chunks(&[FIRST]);
    process_streaming_response(&mut responses, &guardrail, None, None, None)
        .await
        .unwrap();
    assert!(log_text(&log).is_empty(), "{}", log_text(&log));
}
