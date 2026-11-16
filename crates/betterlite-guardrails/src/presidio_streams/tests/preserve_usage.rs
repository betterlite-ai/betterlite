#![cfg(test)]

use rstest::rstest;
use serde_json::value::RawValue;

use crate::presidio_response::wire::ChatModelResponse;
use crate::presidio_streams::unmask::preserve_usage_from_last_chunk;

#[rstest]
#[case::copied_from_the_last_chunk(r#"{"choices":[]}"#, &[r#"{"a":1}"#, r#"{"usage":{"total_tokens":7}}"#], Some(r#"{"total_tokens":7}"#))]
#[case::the_assembled_usage_wins(r#"{"usage":{"total_tokens":1}}"#, &[r#"{"usage":{"total_tokens":7}}"#], Some(r#"{"total_tokens":1}"#))]
#[case::a_null_usage_is_replaced(r#"{"usage":null}"#, &[r#"{"usage":{"total_tokens":7}}"#], Some(r#"{"total_tokens":7}"#))]
#[case::a_null_chunk_usage_is_not_copied(r#"{"choices":[]}"#, &[r#"{"usage":null}"#], None)]
#[case::only_the_last_chunk_counts(r#"{"choices":[]}"#, &[r#"{"usage":{"total_tokens":7}}"#, r#"{"b":2}"#], None)]
#[case::no_chunks(r#"{"choices":[]}"#, &[], None)]
#[case::a_chunk_that_is_not_an_object(r#"{"choices":[]}"#, &["[1]"], None)]
fn preserve_usage_cases(
    #[case] assembled: &str,
    #[case] chunks: &[&str],
    #[case] expected: Option<&str>,
) {
    let mut response: ChatModelResponse = serde_json::from_str(assembled).unwrap();
    let chunks: Vec<Box<RawValue>> = chunks
        .iter()
        .map(|chunk| RawValue::from_string((*chunk).to_owned()).unwrap())
        .collect();
    preserve_usage_from_last_chunk(&mut response, &chunks);
    let usage = response.extra.get("usage").map(|raw| raw.get().to_owned());
    assert_eq!(usage.as_deref(), expected);
}
