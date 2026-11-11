use rstest::rstest;

use super::super::reasoning::parse_content_for_reasoning;

#[rstest]
#[case::plain("hello", (None, "hello"))]
#[case::empty("", (None, ""))]
#[case::think("<think>a</think>b", (Some("a"), "b"))]
#[case::thinking_tag("<thinking>a</thinking>b", (Some("a"), "b"))]
#[case::budget_tag(
    "<budget:thinking>a</budget:thinking>b",
    (Some("a"), "b")
)]
#[case::mixed_tags("<think>a</thinking>b", (Some("a"), "b"))]
#[case::empty_reasoning("<think></think>b", (Some(""), "b"))]
#[case::nothing_after("<think>a</think>", (Some("a"), ""))]
#[case::multiline("<think>a\nb</think>c\nd", (Some("a\nb"), "c\nd"))]
#[case::no_close("<think>abc", (None, "<think>abc"))]
#[case::leading_space(" <think>a</think>b", (None, " <think>a</think>b"))]
#[case::two_tags(
    "<think>a</think>b<think>c</think>d",
    (Some("a"), "b<think>c</think>d")
)]
#[case::close_inside_close("<think>a</think></thinking>b", (Some("a"), "</thinking>b"))]
fn parse_content_for_reasoning_matches_litellm(
    #[case] text: &str,
    #[case] expected: (Option<&str>, &str),
) {
    assert_eq!(parse_content_for_reasoning(text), expected);
}
