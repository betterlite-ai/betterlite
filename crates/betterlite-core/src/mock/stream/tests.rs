#![cfg(test)]

use betterlite_config::sections::mock_response::UsageDefaults;
use rstest::rstest;

use super::{StreamSpec, chunks, usage_chunk};

fn spec(content: &str, choice_count: Option<u32>) -> StreamSpec<'_> {
    StreamSpec {
        content,
        choice_count,
        model: "gpt-4o",
        id: "chatcmpl-test",
        created: 5,
    }
}

#[rstest]
#[case::slices_in_threes("abcdefg", None, &["abc", "def", "g"], 1)]
#[case::short_content("ab", None, &["ab"], 1)]
#[case::empty_content("", None, &[], 1)]
#[case::many_choices("abcd", Some(2), &["abc", "d"], 2)]
#[case::no_choices("abc", Some(0), &["abc"], 0)]
#[case::unicode("h\u{e9}llo", None, &["h\u{e9}l", "lo"], 1)]
fn mock_calls_behavior_tests_behavior(
    #[case] content: &str,
    #[case] choice_count: Option<u32>,
    #[case] expected: &[&str],
    #[case] expected_choices: usize,
) {
    let spec = spec(content, choice_count);
    let collected: Vec<_> = chunks(&spec).collect();
    assert_eq!(collected.len(), expected.len());
    for (chunk, expected_content) in collected.iter().zip(expected) {
        assert_eq!(chunk.choices.len(), expected_choices);
        for choice in &chunk.choices {
            assert_eq!(choice.delta.role.as_deref(), Some("assistant"));
            assert_eq!(choice.delta.content.as_deref(), Some(*expected_content));
        }
        assert_eq!(chunk.id, "chatcmpl-test");
        assert_eq!(chunk.created, 5);
        assert_eq!(chunk.model, "gpt-4o");
        assert_eq!(chunk.object, "chat.completion.chunk");
        assert_eq!(chunk.usage, None);
    }
}

#[test]
fn the_usage_chunk_reports_the_injected_prompt_count() {
    let spec = spec("abc", None);
    let chunk = usage_chunk(
        &spec,
        7,
        UsageDefaults {
            prompt_tokens: 10,
            completion_tokens: 20,
        },
    );
    assert!(chunk.choices.is_empty());
    assert_eq!(chunk.model, "gpt-4o");
    assert_eq!(chunk.id, "chatcmpl-test");
    assert_eq!(chunk.object, "chat.completion.chunk");
    let usage = chunk.usage.expect("usage");
    assert_eq!(usage.prompt_tokens, 7);
    assert_eq!(usage.completion_tokens, 20);
    assert_eq!(usage.total_tokens, 27);
}
