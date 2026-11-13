#![cfg(test)]

use rstest::rstest;

use super::output_cap::is_embedding_request;
use super::support::request;

#[rstest]
#[case::embedding("embedding")]
#[case::async_embedding("aembedding")]
fn embedding_call_types_are_embeddings(#[case] call_type: &str) {
    let data = request(r#"{"input":"hello"}"#);
    assert!(is_embedding_request(&data, Some(call_type)));
}

#[rstest]
#[case::responses("responses")]
#[case::async_responses("aresponses")]
#[case::completion("completion")]
#[case::google("generate_content")]
fn other_call_types_are_never_embeddings(#[case] call_type: &str) {
    let data = request(r#"{"input":"hello"}"#);
    assert!(!is_embedding_request(&data, Some(call_type)));
}

#[test]
fn without_a_call_type_an_input_field_means_embedding() {
    assert!(is_embedding_request(&request(r#"{"input":"hello"}"#), None));
    assert!(is_embedding_request(&request(r#"{"input":""}"#), None));
    assert!(is_embedding_request(&request(r#"{"input":[]}"#), None));
}

#[test]
fn without_a_call_type_an_absent_or_null_input_is_not_an_embedding() {
    assert!(!is_embedding_request(&request("{}"), None));
    assert!(!is_embedding_request(&request(r#"{"input":null}"#), None));
}

#[test]
fn an_embedding_call_type_wins_over_other_fields() {
    assert!(is_embedding_request(&request("{}"), Some("embedding")));
}
