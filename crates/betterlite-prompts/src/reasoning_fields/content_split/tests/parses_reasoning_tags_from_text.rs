use super::*;
use rstest::rstest;

#[rstest]
#[case::think_tag(
    Some("<think>because</think>visible"),
    Some("because"),
    Some("visible")
)]
#[case::thinking_tag(
    Some("<thinking>because</thinking>visible"),
    Some("because"),
    Some("visible")
)]
#[case::budget_thinking_tag(
    Some("<budget:thinking>because</budget:thinking>visible"),
    Some("because"),
    Some("visible")
)]
#[case::multiline_reasoning_body(
    Some("<think>line one\nline two</think>visible"),
    Some("line one\nline two"),
    Some("visible")
)]
#[case::no_tag_at_all(Some("visible"), None, Some("visible"))]
#[case::tag_not_at_the_start(
    Some("prefix<think>because</think>"),
    None,
    Some("prefix<think>because</think>")
)]
#[case::empty_string(Some(""), None, Some(""))]
#[case::none_input(None, None, None)]
#[case::second_closing_tag_stays_in_content(
    Some("<think>a</think>b</think>c"),
    Some("a"),
    Some("b</think>c")
)]
fn reasoning_fields_behavior_tests_behavior(
    #[case] message_text: Option<&str>,
    #[case] expected_reasoning: Option<&str>,
    #[case] expected_content: Option<&str>,
) {
    let (reasoning, content) = parse_content_for_reasoning(message_text);
    assert_eq!(reasoning, expected_reasoning.map(str::to_owned));
    assert_eq!(content, expected_content.map(str::to_owned));
}
