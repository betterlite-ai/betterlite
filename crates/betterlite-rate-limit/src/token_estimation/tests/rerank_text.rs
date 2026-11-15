#![cfg(test)]

use rstest::rstest;

use super::input_text::rerank_input_to_text;
use super::support::request;

#[rstest]
#[case::empty("{}", "")]
#[case::query_only(r#"{"query":"find me"}"#, "find me")]
#[case::documents_without_a_query(r#"{"documents":["one","two"]}"#, "one\ntwo")]
#[case::query_and_documents(
    r#"{"query":"find me","documents":["one","two"]}"#,
    "find me\none\ntwo"
)]
#[case::documents_not_a_list(r#"{"query":"find me","documents":"one"}"#, "find me")]
#[case::non_text_parts_are_skipped(
    r#"{"query":"find me","documents":[1,true,"two"]}"#,
    "find me\ntwo"
)]
#[case::object_documents_use_standard_json(
    r#"{"documents":[{"text":"one"}]}"#,
    r#"{"text":"one"}"#
)]
fn the_rerank_text_joins_text_parts_with_newlines(#[case] body: &str, #[case] expected: &str) {
    assert_eq!(rerank_input_to_text(&request(body)), expected);
}
