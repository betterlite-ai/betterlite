use super::support::data;
use crate::panw_prisma_airs::hooks::prompt::extract_prompt_from_request;

#[rstest::rstest]
#[case::last_user_message_wins(
    r#"{"messages":[{"role":"user","content":"one"},{"role":"assistant","content":"two"},{"role":"user","content":"three"}]}"#,
    "three"
)]
#[case::developer_message_counts(
    r#"{"messages":[{"role":"user","content":"one"},{"role":"developer","content":"dev"}]}"#,
    "dev"
)]
#[case::parts_join_with_spaces(
    r#"{"messages":[{"role":"user","content":[{"type":"text","text":"a"},{"type":"image_url"},{"type":"text","text":"b"}]}]}"#,
    "a b"
)]
#[case::prompt_string_fallback(r#"{"prompt":"hi"}"#, "hi")]
#[case::prompt_list_fallback(r#"{"prompt":["a","","b",null]}"#, "a b")]
#[case::messages_win_over_prompt(
    r#"{"messages":[{"role":"user","content":"m"}],"prompt":"p"}"#,
    "m"
)]
#[case::an_empty_last_user_message_is_skipped(
    r#"{"messages":[{"role":"user","content":"x"},{"role":"user","content":""}]}"#,
    "x"
)]
#[case::non_array_messages_fall_back(r#"{"messages":"hi","prompt":"p"}"#, "p")]
#[case::nothing_found_is_empty("{}", "")]
#[case::null_prompt_is_empty(r#"{"prompt":null}"#, "")]
fn extracts_the_prompt(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(
        extract_prompt_from_request(&data(source)).expect("prompt"),
        expected
    );
}

#[test]
fn a_non_text_prompt_item_names_its_index() {
    let error = extract_prompt_from_request(&data(r#"{"prompt":["a",3]}"#)).expect_err("item");
    assert_eq!(error.to_string(), "item 1 of prompt is not text");
}

#[test]
fn a_non_object_message_names_its_index() {
    let error = extract_prompt_from_request(&data(r#"{"messages":["hi"]}"#)).expect_err("message");
    assert!(
        error
            .to_string()
            .starts_with("item 0 of messages is not the expected object")
    );
}
