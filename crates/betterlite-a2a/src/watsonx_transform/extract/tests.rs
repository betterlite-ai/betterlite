#![cfg(test)]

use rstest::rstest;

use super::{
    WxoRunResult, WxoSendParams, extract_text_from_a2a_params, extract_text_from_wxo_result,
};

fn params_text(json: &str) -> String {
    extract_text_from_a2a_params(&serde_json::from_str::<WxoSendParams>(json).unwrap())
}

fn result_text(json: &str) -> String {
    let run = serde_json::from_str::<WxoRunResult>(json).unwrap_or_default();
    extract_text_from_wxo_result(&run)
}

#[rstest]
#[case::single(
    r#"{"message": {"parts": [{"kind": "text", "text": "hello"}]}}"#,
    "hello"
)]
#[case::joined_with_a_space(
    r#"{"message": {"parts": [{"kind": "text", "text": "a"}, {"text": "b"}, {"kind": "", "text": "c"}, {"kind": null, "text": "d"}]}}"#,
    "a b c d"
)]
#[case::other_kinds_are_skipped(
    r#"{"message": {"parts": [{"kind": "file", "text": "no"}, {"kind": "data", "text": "no"}, {"kind": "text", "text": "yes"}]}}"#,
    "yes"
)]
#[case::empty_text_is_skipped(
    r#"{"message": {"parts": [{"kind": "text", "text": ""}, {"kind": "text", "text": "x"}, {"kind": "text"}]}}"#,
    "x"
)]
#[case::non_object_parts_are_skipped(
    r#"{"message": {"parts": ["text", 5, null, {"text": "ok"}]}}"#,
    "ok"
)]
#[case::no_text_at_all(r#"{"message": {"parts": [{"kind": "file"}]}}"#, "")]
#[case::no_parts(r#"{"message": {}}"#, "")]
#[case::null_parts(r#"{"message": {"parts": null}}"#, "")]
#[case::no_message("{}", "")]
#[case::null_message(r#"{"message": null}"#, "")]
fn a2a_params_text_joins_text_parts_with_spaces(#[case] json: &str, #[case] expected: &str) {
    assert_eq!(params_text(json), expected);
}

#[rstest]
#[case::last_message_first(
    r#"{"last_message": {"content": [{"text": "last"}]},
        "result": {"data": {"message": {"content": [{"text": "nested"}]}}}, "results": "raw"}"#,
    "last"
)]
#[case::only_the_first_content_item_counts(
    r#"{"last_message": {"content": [{"text": ""}, {"text": "second"}]}, "results": "raw"}"#,
    "raw"
)]
#[case::empty_last_message_falls_through_to_nested(
    r#"{"last_message": {"content": [{"text": ""}]},
        "result": {"data": {"message": {"content": [{"text": "nested"}]}}}}"#,
    "nested"
)]
#[case::nested(
    r#"{"result": {"data": {"message": {"content": [{"text": "nested"}]}}}, "results": "raw"}"#,
    "nested"
)]
#[case::empty_nested_falls_through_to_raw(
    r#"{"result": {"data": {"message": {"content": []}}}, "results": "raw"}"#,
    "raw"
)]
#[case::raw_string(r#"{"results": "plain answer"}"#, "plain answer")]
#[case::raw_results_must_be_text(r#"{"results": ["a"]}"#, "")]
#[case::empty_raw(r#"{"results": ""}"#, "")]
#[case::empty_object("{}", "")]
#[case::wrong_shapes_are_skipped(
    r#"{"last_message": "oops", "result": {"data": [1]}, "results": "raw"}"#,
    "raw"
)]
#[case::content_that_is_not_a_list(r#"{"last_message": {"content": {"0": {"text": "x"}}}}"#, "")]
#[case::content_item_without_text(
    r#"{"last_message": {"content": [{"response_type": "text"}]}}"#,
    ""
)]
#[case::content_item_that_is_not_an_object(r#"{"last_message": {"content": ["x"]}}"#, "")]
#[case::not_an_object(r#"["last_message"]"#, "")]
fn wxo_result_text_follows_the_location_priority(#[case] json: &str, #[case] expected: &str) {
    assert_eq!(result_text(json), expected);
}
