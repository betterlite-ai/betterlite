#![cfg(test)]

use rstest::rstest;

use super::{text_rewrites_by_choice, write_ended_text_rewrites};
use crate::guardrails::chat_guardrail_translation::rewrites::string_choice_contents::string_choice_contents;
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::{chunks, response};

const STREAM: &str = r#"[
    {"choices": [{"index": 0, "delta": {"role": "assistant", "content": "He"}}]},
    {"choices": [{"index": 0, "delta": {"content": "llo"}}]},
    {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}]"#;

#[rstest]
#[case::changed(r#"{"choices": [{"index": 0, "message": {"content": "b"}}]}"#, &[Some("a")], &[(0, "b")])]
#[case::unchanged(r#"{"choices": [{"index": 0, "message": {"content": "a"}}]}"#, &[Some("a")], &[])]
#[case::nothing_before(r#"{"choices": [{"index": 0, "message": {"content": "b"}}]}"#, &[None], &[])]
#[case::nothing_after(r#"{"choices": [{"index": 0, "message": {"content": null}}]}"#, &[Some("a")], &[])]
#[case::blanked(r#"{"choices": [{"index": 0, "message": {"content": ""}}]}"#, &[Some("a")], &[(0, "")])]
#[case::keyed_by_the_choice_index(
    r#"{"choices": [{"index": 2, "message": {"content": "x"}}, {"index": 5, "message": {"content": "y"}}]}"#,
    &[Some("a"), Some("b")],
    &[(2, "x"), (5, "y")]
)]
#[case::only_the_changed_choice(
    r#"{"choices": [{"index": 0, "message": {"content": "a"}}, {"index": 1, "message": {"content": "y"}}]}"#,
    &[Some("a"), Some("b")],
    &[(1, "y")]
)]
#[case::later_choice_with_the_same_index_wins(
    r#"{"choices": [{"index": 0, "message": {"content": "x"}}, {"index": 0, "message": {"content": "z"}}]}"#,
    &[Some("a"), Some("b")],
    &[(0, "z")]
)]
fn write_text_rewrites_keep_one_rewrite_per_changed_string_choice(
    #[case] guardrailed: &str,
    #[case] before: &[Option<&str>],
    #[case] expected: &[(i64, &str)],
) {
    let pre: Vec<Option<String>> = before.iter().map(|text| text.map(str::to_owned)).collect();
    let rewrites = text_rewrites_by_choice(&response(guardrailed), &pre);
    let wanted: Vec<(i64, String)> = expected
        .iter()
        .map(|&(index, text)| (index, text.to_owned()))
        .collect();
    assert_eq!(rewrites.into_iter().collect::<Vec<_>>(), wanted);
}

fn rewrite(stream: &str, guardrailed: &str) -> Vec<String> {
    let mut stream_chunks = chunks(stream);
    let guardrailed_response = response(guardrailed);
    let before = string_choice_contents(&response(
        r#"{"choices": [{"index": 0, "message": {"content": "Hello"}}, {"index": 1, "message": {"content": "World"}}]}"#,
    ));
    let pre = &before[..guardrailed_response.choices.len()];
    write_ended_text_rewrites(&mut stream_chunks, &guardrailed_response, pre).unwrap();
    stream_chunks
        .iter()
        .map(|chunk| serde_json::to_string(chunk).unwrap())
        .collect()
}

#[test]
fn write_text_rewrites_put_the_full_text_in_the_first_chunk_and_blank_the_rest() {
    let written = rewrite(
        STREAM,
        r#"{"choices": [{"index": 0, "message": {"content": "HELLO THERE"}}]}"#,
    );
    assert!(written[0].contains(r#""content":"HELLO THERE""#));
    assert!(written[1].contains(r#""content":"""#));
    assert!(!written[2].contains("content"));
}

#[test]
fn write_text_rewrites_leave_the_stream_alone_when_the_text_is_unchanged() {
    let written = rewrite(
        STREAM,
        r#"{"choices": [{"index": 0, "message": {"content": "Hello"}}]}"#,
    );
    assert!(written[0].contains(r#""content":"He""#));
    assert!(written[1].contains(r#""content":"llo""#));
}

#[test]
fn write_text_rewrites_match_choices_by_their_index_not_their_position() {
    let stream = r#"[
        {"choices": [{"index": 1, "delta": {"content": "W"}}]},
        {"choices": [{"index": 0, "delta": {"content": "He"}}]},
        {"choices": [{"index": 1, "delta": {"content": "orld"}}]},
        {"choices": [{"index": 0, "delta": {"content": "llo"}}]}]"#;
    let written = rewrite(
        stream,
        r#"{"choices": [{"index": 0, "message": {"content": "A"}}, {"index": 1, "message": {"content": "B"}}]}"#,
    );
    assert!(written[0].contains(r#""content":"B""#));
    assert!(written[1].contains(r#""content":"A""#));
    assert!(written[2].contains(r#""content":"""#));
    assert!(written[3].contains(r#""content":"""#));
}

#[test]
fn write_text_rewrites_give_the_whole_text_to_a_first_chunk_that_carries_empty_content() {
    let stream = r#"[
        {"choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}}]},
        {"choices": [{"index": 0, "delta": {"content": "Hello"}}]}]"#;
    let written = rewrite(
        stream,
        r#"{"choices": [{"index": 0, "message": {"content": "BYE"}}]}"#,
    );
    assert!(written[0].contains(r#""content":"BYE""#));
    assert!(written[1].contains(r#""content":"""#));
}
