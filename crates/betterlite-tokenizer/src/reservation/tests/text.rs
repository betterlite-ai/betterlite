use rstest::rstest;

use crate::reservation::{approximate_input_size, count_input_tokens_for_model};

use super::support;

const MODEL: &str = "gpt-3.5-turbo";

#[rstest]
#[case::messages_null_blocks_prompt(r#"{"messages": null, "prompt": "hello world"}"#, Some(3))]
#[case::messages_empty_blocks_prompt(r#"{"messages": [], "prompt": "hello world"}"#, Some(3))]
#[case::messages_precede_prompt(
    r#"{"messages": [{"role": "user", "content": "hello world"}], "prompt": "hello world"}"#,
    Some(9)
)]
#[case::prompt_precedes_input(
    r#"{"prompt": "hello world", "input": "hello world hello world"}"#,
    Some(2)
)]
#[case::input_without_prompt(r#"{"input": "hello world"}"#, Some(2))]
#[case::query_and_documents_add(r#"{"query": "hello world", "documents": "hello world"}"#, Some(4))]
#[case::documents_without_query(r#"{"documents": "hello world"}"#, Some(2))]
#[case::false_messages_blocks_prompt(r#"{"messages": false, "prompt": "hello world"}"#, Some(3))]
#[case::zero_messages_blocks_prompt(r#"{"messages": 0, "prompt": "hello world"}"#, Some(3))]
#[case::zero_float_messages_blocks_prompt(r#"{"messages": 0.0, "prompt": "hello world"}"#, Some(3))]
#[case::number_messages_is_unavailable(r#"{"messages": 5}"#, None)]
#[case::float_messages_is_unavailable(r#"{"messages": 5.0}"#, None)]
#[case::null_prompt_is_zero(r#"{"prompt": null}"#, Some(0))]
#[case::null_input_is_zero(r#"{"input": null}"#, Some(0))]
#[case::no_recognized_field(r#"{"model": "gpt-4", "stream": true}"#, None)]
#[tokio::test]
async fn field_presence_decides_the_count(#[case] source: &str, #[case] expected: Option<usize>) {
    let directory = tempfile::tempdir().unwrap();
    let counter = support::bundled_counter(directory.path());
    let body = support::body(source);
    assert_eq!(
        count_input_tokens_for_model(&counter, &body, MODEL).await,
        expected
    );
}

#[tokio::test]
async fn text_counting_matches_python_str_and_json_dumps() {
    let directory = tempfile::tempdir().unwrap();
    let counter = support::bundled_counter(directory.path());
    let body = support::body(
        "{\"prompt\": [null, true, false, 2, \"hi\", [3, {\"b\": true, \"a\": \"\u{e9}\"}], {\"k\": \"it's\"}]}",
    );
    let expected: usize = [
        support::text_tokens(&counter, MODEL, "True").await,
        support::text_tokens(&counter, MODEL, "False").await,
        support::text_tokens(&counter, MODEL, "2").await,
        support::text_tokens(&counter, MODEL, "hi").await,
        support::text_tokens(&counter, MODEL, "3").await,
        support::text_tokens(&counter, MODEL, "{\"b\": true, \"a\": \"\\u00e9\"}").await,
        support::text_tokens(&counter, MODEL, "{\"k\": \"it's\"}").await,
    ]
    .into_iter()
    .sum();
    assert_eq!(
        count_input_tokens_for_model(&counter, &body, MODEL).await,
        Some(expected)
    );
}

#[test]
fn the_size_estimate_sums_python_str_of_every_counted_field() {
    let body = support::body(
        "{\"messages\": [{\"a\": true}], \"prompt\": \"h\u{e9}llo\", \"input\": null, \"documents\": [], \"tools\": true, \"tool_choice\": {\"x\": 1}}",
    );
    // [{'a': True}] is 13, h\u{e9}llo is 5 code points, None 4, [] 2, True 4 and {'x': 1} 8
    assert_eq!(approximate_input_size(&body), 36);
}

#[test]
fn the_size_estimate_counts_mapping_keys_and_unicode_scalars() {
    let body = support::body("{\"tools\": [{\"name\": \"\u{1f600}\", \"it's\": \"x\"}]}");
    let rendered = "[{'name': '\u{1f600}', \"it's\": 'x'}]";
    assert_eq!(approximate_input_size(&body), rendered.chars().count());
}
