#![cfg(test)]

mod support;

use rstest::rstest;
use serde::Deserialize;

use super::{
    A2aRequest, RequestUtilsError, count_tokens, extract_text_from_message,
    extract_text_from_response, get_input_message_from_request, usage_from_request_response,
};
use crate::a2a_text::{A2aResponse, A2aTextError, RecordValue};
use support::{counter, failing_counter};

#[derive(Deserialize)]
struct Probe {
    message: Option<RecordValue>,
    response: Option<A2aResponse>,
    expect: Outcome,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Text(String),
    Error(String),
}

fn assert_projection(source: &str) {
    let probe: Probe = serde_json::from_str(source).expect("the fixture parses");
    let actual = match &probe.response {
        Some(response) => extract_text_from_response(response),
        None => extract_text_from_message(probe.message.as_ref()),
    };
    let outcome = actual.map_or_else(|error| Outcome::Error(format!("{error:?}")), Outcome::Text);
    assert_eq!(outcome, probe.expect);
}

#[rstest]
#[case::text_parts_join_with_one_space(r#"{"expect":{"text":"one two"},"message":{"parts":[{"kind":"text","text":"one"},{"kind":"text","text":"two"}]}}"#)]
#[case::missing_text_adds_an_empty_part(r#"{"expect":{"text":"one  two"},"message":{"parts":[{"kind":"text","text":"one"},{"kind":"text"},{"kind":"text","text":"two"}]}}"#)]
#[case::non_text_kinds_are_skipped(r#"{"expect":{"text":"y"},"message":{"parts":[{"kind":"data","text":"x"},{"kind":"text","text":"y"},{"role":"user","text":"z"}]}}"#)]
#[case::nested_parts_are_not_followed(r#"{"expect":{"text":""},"message":{"parts":[{"kind":"data","parts":[{"kind":"text","text":"inner"}]}]}}"#)]
#[case::the_kind_must_be_the_exact_word(r#"{"expect":{"text":""},"message":{"parts":[{"kind":"TEXT","text":"x"},{"kind":"text-message","text":"y"}]}}"#)]
#[case::an_empty_parts_list_is_empty(r#"{"expect":{"text":""},"message":{"parts":[]}}"#)]
#[case::missing_parts_is_empty(r#"{"expect":{"text":""},"message":{"role":"user"}}"#)]
#[case::a_missing_message_is_empty(r#"{"expect":{"text":""},"message":null}"#)]
#[case::a_non_object_message_is_empty(r#"{"expect":{"text":""},"message":"text"}"#)]
#[case::non_object_parts_are_skipped(
    r#"{"expect":{"text":"x"},"message":{"parts":["text",5,null,{"kind":"text","text":"x"}]}}"#
)]
#[case::a_text_parts_value_is_iterated_as_skips(
    r#"{"expect":{"text":""},"message":{"parts":"abc"}}"#
)]
#[case::an_object_parts_value_is_iterated_as_skips(
    r#"{"expect":{"text":""},"message":{"parts":{"kind":"text","text":"x"}}}"#
)]
fn text_extraction_cases(#[case] source: &str) {
    assert_projection(source);
}

#[rstest]
#[case::a_null_parts_value_is_not_iterable(
    r#"{"expect":{"error":"PartsNotIterable"},"message":{"parts":null}}"#
)]
#[case::a_boolean_parts_value_is_not_iterable(
    r#"{"expect":{"error":"PartsNotIterable"},"message":{"parts":false}}"#
)]
#[case::a_number_parts_value_is_not_iterable(
    r#"{"expect":{"error":"PartsNotIterable"},"message":{"parts":2}}"#
)]
#[case::a_null_text_is_not_a_string(
    r#"{"expect":{"error":"TextNotString"},"message":{"parts":[{"kind":"text","text":null}]}}"#
)]
#[case::a_number_text_is_not_a_string(
    r#"{"expect":{"error":"TextNotString"},"message":{"parts":[{"kind":"text","text":5}]}}"#
)]
#[case::a_list_text_is_not_a_string(
    r#"{"expect":{"error":"TextNotString"},"message":{"parts":[{"kind":"text","text":["a"]}]}}"#
)]
fn malformed_text_cases(#[case] source: &str) {
    assert_projection(source);
}

#[rstest]
#[case::a_direct_message_is_extracted(r#"{"expect":{"text":"direct"},"response":{"result":{"kind":"message","parts":[{"kind":"text","text":"direct"}]}}}"#)]
#[case::a_direct_message_wins_over_its_nested_message(r#"{"expect":{"text":"direct"},"response":{"result":{"kind":"message","parts":[{"kind":"text","text":"direct"}],"message":{"parts":[{"kind":"text","text":"nested"}]}}}}"#)]
#[case::a_nested_message_is_extracted(r#"{"expect":{"text":"nested"},"response":{"result":{"message":{"parts":[{"kind":"text","text":"nested"}]}}}}"#)]
#[case::a_task_result_extracts_its_nested_message(r#"{"expect":{"text":"nested"},"response":{"result":{"kind":"task","message":{"parts":[{"kind":"text","text":"nested"}]}}}}"#)]
#[case::result_parts_without_the_message_kind_are_ignored(
    r#"{"expect":{"text":""},"response":{"result":{"parts":[{"kind":"text","text":"direct"}]}}}"#
)]
#[case::a_missing_result_is_empty(r#"{"expect":{"text":""},"response":{}}"#)]
#[case::a_null_result_is_empty(r#"{"expect":{"text":""},"response":{"result":null}}"#)]
#[case::a_list_result_is_empty(r#"{"expect":{"text":""},"response":{"result":[{"kind":"message","parts":[{"kind":"text","text":"x"}]}]}}"#)]
#[case::a_null_message_is_empty(r#"{"expect":{"text":""},"response":{"result":{"message":null}}}"#)]
#[case::a_non_object_message_is_empty(
    r#"{"expect":{"text":""},"response":{"result":{"message":5}}}"#
)]
#[case::a_direct_message_without_parts_is_empty(
    r#"{"expect":{"text":""},"response":{"result":{"kind":"message"}}}"#
)]
#[case::the_jsonrpc_envelope_is_ignored(r#"{"expect":{"text":"x"},"response":{"jsonrpc":"2.0","id":1,"result":{"kind":"message","parts":[{"kind":"text","text":"x"}]}}}"#)]
fn response_extraction_cases(#[case] source: &str) {
    assert_projection(source);
}

#[rstest]
#[case::a_null_response_text_is_not_a_string(r#"{"expect":{"error":"TextNotString"},"response":{"result":{"kind":"message","parts":[{"kind":"text","text":null}]}}}"#)]
#[case::malformed_nested_message(
    r#"{"expect":{"error":"PartsNotIterable"},"response":{"result":{"message":{"parts":true}}}}"#
)]
fn malformed_response_cases(#[case] source: &str) {
    assert_projection(source);
}

#[rstest]
#[case::a_params_message(r#"{"params":{"message":{"parts":[]}}}"#, true)]
#[case::no_params("{}", false)]
#[case::no_message(r#"{"params":{}}"#, false)]
#[case::null_params(r#"{"params":null}"#, false)]
#[case::non_object_params(r#"{"params":[]}"#, false)]
fn the_input_message_is_read_from_the_request(#[case] source: &str, #[case] present: bool) {
    let request: A2aRequest = serde_json::from_str(source).expect("the fixture parses");

    assert_eq!(get_input_message_from_request(&request).is_some(), present);
}

#[tokio::test]
async fn counts_the_default_encoding_tokens() {
    assert_eq!(count_tokens(&counter(), "hello world").await, 2);
}

#[tokio::test]
async fn empty_text_is_zero_tokens() {
    assert_eq!(count_tokens(&failing_counter(), "").await, 0);
}

#[tokio::test]
async fn a_failing_tokenizer_is_zero_tokens() {
    assert_eq!(count_tokens(&failing_counter(), "hello world").await, 0);
}

#[derive(Deserialize)]
struct UsageProbe {
    request: A2aRequest,
    response: A2aResponse,
}

fn usage_probe(source: &str) -> UsageProbe {
    serde_json::from_str(source).expect("the fixture parses")
}

#[rstest]
#[case::explicit_kind(
    r#"{"request":{"params":{"message":{"parts":[{"kind":"text","text":"hello world"}]}}},"response":{"result":{"kind":"message","parts":[{"kind":"text","text":"hello world"}]}}}"#
)]
#[case::sdk_default_kind(
    r#"{"request":{"params":{"message":{"parts":[{"text":"hello world"}]}}},"response":{"result":{"kind":"message","parts":[{"kind":"text","text":"hello world"}]}}}"#
)]
#[tokio::test]
async fn usage_counts_prompt_and_completion_text(#[case] source: &str) {
    let probe = usage_probe(source);

    let usage = usage_from_request_response(&counter(), &probe.request, &probe.response)
        .await
        .expect("the fixture extracts");

    assert_eq!(usage, (2, 2, 4));
}

#[tokio::test]
async fn usage_without_any_text_is_zero() {
    let probe = usage_probe(r#"{"request":{"params":{}},"response":{"result":{}}}"#);

    let usage = usage_from_request_response(&counter(), &probe.request, &probe.response)
        .await
        .expect("the fixture extracts");

    assert_eq!(usage, (0, 0, 0));
}

#[rstest]
#[case::no_params(
    r#"{"request":{},"response":{"result":{"kind":"message","parts":[{"kind":"text","text":"hello world"}]}}}"#
)]
#[case::no_message(r#"{"request":{"params":{}},"response":{"result":{"kind":"message","parts":[{"kind":"text","text":"hello world"}]}}}"#)]
#[tokio::test]
async fn usage_without_a_request_message_counts_no_prompt(#[case] source: &str) {
    let probe = usage_probe(source);

    let usage = usage_from_request_response(&counter(), &probe.request, &probe.response)
        .await
        .expect("the fixture extracts");

    assert_eq!(usage, (0, 2, 2));
}

#[tokio::test]
async fn usage_with_a_failing_tokenizer_is_zero() {
    let probe = usage_probe(
        r#"{"request":{"params":{"message":{"parts":[{"kind":"text","text":"hello world"}]}}},"response":{"result":{"kind":"message","parts":[{"kind":"text","text":"hello world"}]}}}"#,
    );

    let usage = usage_from_request_response(&failing_counter(), &probe.request, &probe.response)
        .await
        .expect("the fixture extracts");

    assert_eq!(usage, (0, 0, 0));
}

#[tokio::test]
async fn usage_propagates_a_malformed_request_message() {
    let probe = usage_probe(
        r#"{"request":{"params":{"message":{"parts":null}}},"response":{"result":{}}}"#,
    );

    let usage = usage_from_request_response(&counter(), &probe.request, &probe.response).await;

    assert_eq!(
        usage,
        Err(RequestUtilsError::Text(A2aTextError::PartsNotIterable))
    );
}

#[tokio::test]
async fn usage_propagates_a_malformed_response() {
    let probe = usage_probe(
        r#"{"request":{"params":{}},"response":{"result":{"message":{"parts":[{"kind":"text","text":null}]}}}}"#,
    );

    let usage = usage_from_request_response(&counter(), &probe.request, &probe.response).await;

    assert_eq!(
        usage,
        Err(RequestUtilsError::Text(A2aTextError::TextNotString))
    );
}
