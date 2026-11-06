#![cfg(test)]

use rstest::rstest;

use super::{PromptData, iter_message_text};

#[rstest]
#[case::message_text_and_parts(
    r#"{"messages":[{"role":"system","content":"sys"},{"role":"user","content":""},{"role":"assistant","content":[{"type":"text","text":"part one"},{"type":"input_text","text":"part two"},{"type":"image_url","image_url":{"url":"u"}}]},{"role":"user","content":null}]}"#,
    vec!["sys", "part one", "part two"]
)]
#[case::bare_strings_inside_a_part_list(
    r#"{"messages":[{"role":"user","content":["bare",{"type":"text","text":"part"}]}]}"#,
    vec!["bare", "part"]
)]
#[case::responses_input_string(r#"{"input":"hello"}"#, vec!["hello"])]
#[case::responses_input_empty_string(r#"{"input":""}"#, vec![])]
#[case::responses_input_items(
    r#"{"input":["plain",{"type":"message","role":"user","content":"body"},{"type":"reasoning","content":"think","summary":["one","two"]},{"type":"function_call_output","output":"tool output"},{"type":"custom_tool_call_output","output":["a","b"]},{"type":"unknown","value":"x"}]}"#,
    vec!["plain", "body", "think", "one", "two", "tool output", "a", "b"]
)]
#[case::item_text_wins_over_content(
    r#"{"input":[{"type":"function_call_output","text":"direct","content":"ignored","output":"ignored too"}]}"#,
    vec!["direct"]
)]
#[case::empty_item_text_falls_through_to_content(
    r#"{"input":[{"text":"","content":"content wins"}]}"#,
    vec!["content wins"]
)]
#[case::reasoning_summary_without_content(
    r#"{"input":[{"type":"reasoning","summary":["s"]}]}"#,
    vec!["s"]
)]
#[case::messages_come_before_input(
    r#"{"messages":[{"role":"user","content":"message"}],"input":"responses"}"#,
    vec!["message", "responses"]
)]
fn iter_message_text_extracts_every_text_fragment(
    #[case] raw: &str,
    #[case] expected_texts: Vec<&str>,
) {
    let data: PromptData = serde_json::from_str(raw).expect("prompt data");
    let expected: Vec<String> = expected_texts.into_iter().map(str::to_owned).collect();
    assert_eq!(iter_message_text(&data), expected);
}
