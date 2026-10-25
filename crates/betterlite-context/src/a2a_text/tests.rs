#![cfg(test)]
use super::{
    A2aResponse, RecordValue, extract_text_from_a2a_message, extract_text_from_a2a_response,
};
use rstest::rstest;
use serde::Deserialize;

#[derive(Deserialize)]
struct Probe {
    message: Option<RecordValue>,
    response: Option<A2aResponse>,
    #[serde(default)]
    depth: usize,
    max_depth: usize,
    expect: Outcome,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Text(String),
    Error(String),
}

fn assert_projection(source: &str) {
    let probe: Probe = serde_json::from_str(source).unwrap();
    let actual = match &probe.response {
        Some(response) => extract_text_from_a2a_response(response, probe.max_depth),
        None => extract_text_from_a2a_message(probe.message.as_ref(), probe.depth, probe.max_depth),
    };
    let outcome = actual.map_or_else(|error| Outcome::Error(format!("{error:?}")), Outcome::Text);
    assert_eq!(outcome, probe.expect);
}

#[rstest]
#[case::a2a_text_parts(r#"{"max_depth":10,"expect":{"text":" one  two"},"message":{"parts":[{"kind":"text","text":""},{"kind":"text","text":"one","parts":[{"kind":"text","text":"no"}]},{"kind":"text"},{"kind":"data","parts":[{"kind":"text","text":"two"}]},{"kind":"data","parts":[]},{"kind":"file"}]}}"#)]
#[case::a2a_depth_limit(r#"{"depth":9,"max_depth":10,"expect":{"text":"top"},"message":{"parts":[{"kind":"text","text":"top"},{"parts":[{"kind":"text","text":"inner"}]}]}}"#)]
#[case::a2a_result_priority(r#"{"max_depth":10,"expect":{"text":""},"response":{"result":{"parts":[],"message":{"parts":[{"kind":"text","text":"later"}]}}}}"#)]
#[case::a2a_wrong_shapes(r#"{"max_depth":10,"expect":{"error":"PartNotObject"},"message":{"parts":[{"kind":"text","text":false},"part"]}}"#)]
fn a2a_text_cases(#[case] source: &str) {
    assert_projection(source);
}

#[rstest]
#[case::none(r#"{"max_depth":10,"expect":{"text":""},"message":null}"#)]
#[case::missing_parts(r#"{"max_depth":10,"expect":{"text":""},"message":{"kind":"message"}}"#)]
#[case::depth_eight(r#"{"depth":8,"max_depth":10,"expect":{"text":"top inner"},"message":{"parts":[{"kind":"text","text":"top"},{"parts":[{"kind":"text","text":"inner"}]}]}}"#)]
#[case::caller_zero_limit(
    r#"{"max_depth":0,"expect":{"text":""},"message":{"parts":[{"kind":"text","text":"top"}]}}"#
)]
#[case::smaller_limit(r#"{"max_depth":1,"expect":{"text":"top"},"message":{"parts":[{"kind":"text","text":"top"},{"parts":[{"kind":"text","text":"inner"}]}]}}"#)]
#[case::non_object_at_limit(r#"{"depth":3,"max_depth":3,"expect":{"text":""},"message":[1]}"#)]
#[case::bad_parts_at_limit(
    r#"{"max_depth":1,"expect":{"text":""},"message":{"parts":[{"parts":5}]}}"#
)]
#[case::empty_string_parts(r#"{"max_depth":10,"expect":{"text":""},"message":{"parts":""}}"#)]
#[case::empty_object_parts(r#"{"max_depth":10,"expect":{"text":""},"message":{"parts":{}}}"#)]
#[case::message_string(
    r#"{"max_depth":10,"expect":{"error":"MessageNotObject"},"message":"text"}"#
)]
#[case::parts_null(
    r#"{"max_depth":10,"expect":{"error":"PartsNotIterable"},"message":{"parts":null}}"#
)]
#[case::parts_flag(
    r#"{"max_depth":10,"expect":{"error":"PartsNotIterable"},"message":{"parts":false}}"#
)]
#[case::parts_string(
    r#"{"max_depth":10,"expect":{"error":"PartNotObject"},"message":{"parts":"bad"}}"#
)]
#[case::parts_object(
    r#"{"max_depth":10,"expect":{"error":"PartNotObject"},"message":{"parts":{"kind":"text"}}}"#
)]
#[case::text_null(r#"{"max_depth":10,"expect":{"error":"TextNotString"},"message":{"parts":[{"kind":"text","text":null}]}}"#)]
#[case::nested_text_list(r#"{"max_depth":10,"expect":{"error":"TextNotString"},"message":{"parts":[{"parts":[{"kind":"text","text":[]}]}]}}"#)]
#[case::list_result(r#"{"max_depth":10,"expect":{"text":""},"response":{"result":[{"parts":[{"kind":"text","text":"x"}]}]}}"#)]
#[case::direct_parts(r#"{"max_depth":10,"expect":{"text":"d"},"response":{"result":{"parts":[{"kind":"text","text":"d"}]}}}"#)]
#[case::direct_parts_zero_limit(r#"{"max_depth":0,"expect":{"text":""},"response":{"result":{"parts":[{"kind":"text","text":"d"}]}}}"#)]
#[case::message_wins(r#"{"max_depth":10,"expect":{"text":"m"},"response":{"result":{"message":{"parts":[{"kind":"text","text":"m"}]},"artifact":{"parts":[{"kind":"text","text":"a"}]}}}}"#)]
#[case::message_role_only(r#"{"max_depth":10,"expect":{"text":""},"response":{"result":{"message":{"role":"agent"},"artifacts":[{"parts":[{"kind":"text","text":"a"}]}]}}}"#)]
#[case::message_kind_null(r#"{"max_depth":10,"expect":{"text":""},"response":{"result":{"message":{"kind":null},"artifacts":[{"parts":[{"kind":"text","text":"a"}]}]}}}"#)]
#[case::message_text_null(r#"{"max_depth":10,"expect":{"text":""},"response":{"result":{"message":{"text":null},"artifacts":[{"parts":[{"kind":"text","text":"a"}]}]}}}"#)]
#[case::message_empty(r#"{"max_depth":10,"expect":{"text":"a"},"response":{"result":{"message":{},"artifact":{"parts":[{"kind":"text","text":"a"}]}}}}"#)]
#[case::message_zero(r#"{"max_depth":10,"expect":{"text":"a"},"response":{"result":{"message":0,"artifact":{"parts":[{"kind":"text","text":"a"}]}}}}"#)]
#[case::message_half(r#"{"max_depth":10,"expect":{"error":"MessageNotObject"},"response":{"result":{"message":0.5}}}"#)]
#[case::message_list(r#"{"max_depth":10,"expect":{"error":"MessageNotObject"},"response":{"result":{"message":[0]}}}"#)]
#[case::artifact_list(r#"{"max_depth":10,"expect":{"text":"s"},"response":{"result":{"artifact":[1],"status":{"message":{"parts":[{"kind":"text","text":"s"}]}}}}}"#)]
#[case::artifact_empty(r#"{"max_depth":10,"expect":{"text":"s"},"response":{"result":{"artifact":{},"status":{"message":{"parts":[{"kind":"text","text":"s"}]}}}}}"#)]
#[case::status_wins(r#"{"max_depth":10,"expect":{"text":"s"},"response":{"result":{"status":{"message":{"parts":[{"kind":"text","text":"s"}]}},"artifacts":[{"parts":[{"kind":"text","text":"a"}]}]}}}"#)]
#[case::status_string(r#"{"max_depth":10,"expect":{"text":"a"},"response":{"result":{"status":"done","artifacts":[{"parts":[{"kind":"text","text":"a"}]}]}}}"#)]
#[case::status_message_empty(r#"{"max_depth":10,"expect":{"text":"a"},"response":{"result":{"status":{"message":{}},"artifacts":[{"parts":[{"kind":"text","text":"a"}]}]}}}"#)]
#[case::first_artifact_only(r#"{"max_depth":10,"expect":{"text":""},"response":{"result":{"artifacts":[{"parts":[]},{"parts":[{"kind":"text","text":"b"}]}]}}}"#)]
#[case::artifacts_string(r#"{"max_depth":10,"expect":{"error":"MessageNotObject"},"response":{"result":{"artifacts":"abc"}}}"#)]
#[case::artifacts_string_zero_limit(
    r#"{"max_depth":0,"expect":{"text":""},"response":{"result":{"artifacts":"abc"}}}"#
)]
#[case::artifacts_empty_string(
    r#"{"max_depth":10,"expect":{"text":""},"response":{"result":{"artifacts":""}}}"#
)]
#[case::artifacts_object(r#"{"max_depth":10,"expect":{"error":"ArtifactsNotIndexed"},"response":{"result":{"artifacts":{"a":1}}}}"#)]
#[case::artifacts_empty_object(
    r#"{"max_depth":10,"expect":{"text":""},"response":{"result":{"artifacts":{}}}}"#
)]
#[case::artifacts_number(r#"{"max_depth":10,"expect":{"error":"ArtifactsNotSized"},"response":{"result":{"artifacts":2}}}"#)]
#[case::artifacts_true(r#"{"max_depth":10,"expect":{"error":"ArtifactsNotSized"},"response":{"result":{"artifacts":true}}}"#)]
#[case::artifacts_zero(
    r#"{"max_depth":10,"expect":{"text":""},"response":{"result":{"artifacts":0}}}"#
)]
fn projection(#[case] source: &str) {
    assert_projection(source);
}
