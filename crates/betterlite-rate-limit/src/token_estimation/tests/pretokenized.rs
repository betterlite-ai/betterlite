#![cfg(test)]

use rstest::rstest;

use super::input_text::pretokenized_embedding_tokens;
use super::support::value;

#[rstest]
#[case::not_a_list(r#""tokens""#, None)]
#[case::number("7", None)]
#[case::object(r#"{"tokens":[1]}"#, None)]
#[case::empty_list("[]", Some(0))]
#[case::flat_ids("[1,2,3]", Some(3))]
#[case::negative_id("[-1]", Some(1))]
#[case::id_past_i64("[18446744073709551615]", Some(1))]
#[case::nested_ids("[[1,2],[3]]", Some(3))]
#[case::nested_empty("[[],[]]", Some(0))]
#[case::mixed("[1,[2]]", None)]
#[case::text(r#"["a"]"#, None)]
#[case::float("[1.5]", None)]
#[case::boolean("[true]", None)]
#[case::nested_text(r#"[["a"]]"#, None)]
#[case::nested_mixed(r#"[[1],"x"]"#, None)]
fn only_integer_token_ids_count(#[case] body: &str, #[case] expected: Option<i64>) {
    assert_eq!(pretokenized_embedding_tokens(&value(body)), expected);
}
