#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{completion_prompt_to_str, is_token_id_prompt};

fn prompt(raw: &str) -> Json {
    serde_json::from_str(raw).unwrap()
}

#[rstest]
#[case::absent(None, None)]
#[case::string(Some(r#""  hello world  ""#), Some("hello world"))]
#[case::blank_string(Some(r#""   ""#), None)]
#[case::empty_list(Some("[]"), None)]
#[case::token_ids(Some("[15496, 995]"), None)]
#[case::string_list(Some(r#"["a ", " b", "c"]"#), Some("a\nb\nc"))]
#[case::blank_string_list(Some(r#"[" ", "  "]"#), None)]
#[case::strings_among_token_ids(Some(r#"[" keep ", 5, "me"]"#), Some("keep\nme"))]
#[case::token_id_batches(Some("[[1, 2], [3]]"), None)]
#[case::strings_among_token_id_batches(Some(r#"["text", [1, 2]]"#), Some("text"))]
#[case::object(Some(r#"{"a": "b"}"#), None)]
fn completion_prompt_to_str_normalizes_the_prompt(
    #[case] raw: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let value = raw.map(prompt);

    assert_eq!(
        completion_prompt_to_str(value.as_ref()).as_deref(),
        expected
    );
}

#[rstest]
#[case::absent(None, false)]
#[case::string(Some(r#""text""#), false)]
#[case::empty_list(Some("[]"), false)]
#[case::strings(Some(r#"["a", "b"]"#), false)]
#[case::flat_token_ids(Some("[1, 2, 3]"), true)]
#[case::batched_token_ids(Some("[[1, 2], [3]]"), true)]
#[case::strings_then_token_id(Some(r#"["a", 7]"#), true)]
#[case::strings_then_token_id_batch(Some(r#"["a", [1, 2]]"#), true)]
#[case::empty_batch(Some("[[]]"), false)]
#[case::float(Some("[1.5]"), false)]
fn is_token_id_prompt_detects_every_token_id_shape(
    #[case] raw: Option<&str>,
    #[case] expected: bool,
) {
    let value = raw.map(prompt);

    assert_eq!(is_token_id_prompt(value.as_ref()), expected);
}
