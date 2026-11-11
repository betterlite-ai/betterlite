#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use indexmap::IndexMap;
use rstest::rstest;

use super::parse_json_verdict;
use crate::decisions::Json;
use crate::judge_primitives::error::JudgePrimitivesError;

fn expected(fields: &str) -> IndexMap<String, Json> {
    match serde_json::from_str::<Json>(fields).unwrap() {
        Json::Object(object) => object,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture is not an object")
        }
    }
}

#[rstest]
#[case::with_json_marker(concat!("```json\n", "{\"score\": 2}\n", "```"), r#"{"score": 2}"#)]
#[case::without_marker(concat!("```\n", "{\"score\": 2}\n", "```"), r#"{"score": 2}"#)]
#[case::marker_in_capitals(concat!("```JSON\n", "{\"score\": 2}\n", "```"), r#"{"score": 2}"#)]
#[case::prose_around_the_fence(
    concat!("Here is my verdict:\n```json\n", "{\"score\": 2}\n", "```\nThanks!"),
    r#"{"score": 2}"#
)]
#[case::first_fence_wins(
    concat!(
        "```json\n", "{\"score\": 2}\n", "```\n",
        "```json\n", "{\"score\": 99}\n", "```"
    ),
    r#"{"score": 2}"#
)]
#[case::outermost_braces_without_a_fence(
    r#"The answer is {"score": 2} as requested"#,
    r#"{"score": 2}"#
)]
#[case::nested_braces_keep_the_outermost_pair(
    r#"prefix {"outer": {"score": 2}} suffix"#,
    r#"{"outer": {"score": 2}}"#
)]
#[case::multiline_json_inside_the_fence(
    concat!("```json\n", "{\n  \"score\": 2\n}\n", "```"),
    r#"{"score": 2}"#
)]
#[case::prose_after_the_braced_object(
    r#"verdict: {"score": 2} and trailing words"#,
    r#"{"score": 2}"#
)]
#[case::fence_payload_falls_back_to_its_braces(
    concat!("```json\n", "verdict {\"score\": 2}\n", "```"),
    r#"{"score": 2}"#
)]
#[case::whitespace_around_the_reply(
    concat!("  \n", "```json\n", "{\"score\": 2}\n", "```\n", "  "),
    r#"{"score": 2}"#
)]
#[case::keeps_the_key_order_as_sent(r#"{"zeta": 1, "alpha": 2}"#, r#"{"zeta": 1, "alpha": 2}"#)]
#[case::keeps_a_decimal_number(r#"{"score": 2.5}"#, r#"{"score": 2.5}"#)]
fn extracts_the_verdict_object(#[case] raw: &str, #[case] fields: &str) {
    assert_eq!(parse_json_verdict(raw).unwrap(), expected(fields));
}

#[rstest]
#[case::array("[1, 2]")]
#[case::fenced_array(concat!("```json\n", "[1, 2]\n", "```"))]
#[case::fenced_array_with_a_capital_marker(concat!("```JSON\n", "[{\"score\": 2}]\n", "```"))]
#[case::multiline_fenced_array(concat!("```json\n", "[\n", "{\"score\": 2}\n", "]\n", "```"))]
#[case::empty_array("[]")]
#[case::string("\"verdict\"")]
#[case::number("42")]
#[case::boolean("true")]
#[case::null("null")]
fn rejects_json_that_is_not_an_object(#[case] raw: &str) {
    assert!(matches!(
        parse_json_verdict(raw),
        Err(JudgePrimitivesError::NotAnObject)
    ));
}

#[rstest]
#[case::no_json_at_all("not a verdict")]
#[case::unclosed_object(r#"{"score": 2"#)]
#[case::empty_reply("")]
#[case::empty_fence(concat!("```json\n", "\n", "```"))]
#[case::empty_fence_without_a_marker("``````")]
#[case::braces_that_do_not_parse("prefix {score: 2} suffix")]
#[case::single_opening_brace("verdict {")]
fn rejects_a_reply_with_no_parseable_json(#[case] raw: &str) {
    assert!(matches!(
        parse_json_verdict(raw),
        Err(JudgePrimitivesError::Json { .. })
    ));
}

#[test]
fn falls_back_to_braces_inside_the_fence_only() {
    let raw = concat!("{\"score\": 7}\n```json\n", "not json\n", "```");
    assert!(matches!(
        parse_json_verdict(raw),
        Err(JudgePrimitivesError::Json { .. })
    ));
}

#[test]
fn an_unclosed_fence_falls_back_to_the_whole_reply() {
    assert_eq!(
        parse_json_verdict("```json\n{\"score\": 2}").unwrap(),
        expected(r#"{"score": 2}"#)
    );
}

#[rstest]
fn replays_the_judge_verdict_parser_corpus(
    #[files("fuzz/corpus/judge_verdict_parser/*")] path: PathBuf,
) {
    let source = fs::read_to_string(path).unwrap();
    drop(parse_json_verdict(&source));
}
