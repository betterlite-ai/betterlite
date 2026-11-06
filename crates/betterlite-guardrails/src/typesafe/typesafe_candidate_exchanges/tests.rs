#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::candidate_exchanges;
use crate::typesafe::MAX_EXCHANGES_EVALUATED;
use crate::typesafe::fixtures::{guardrail, json, settings};
use crate::typesafe::typesafe_config::MinCharsToEvaluate;

fn completed_exchange() -> Vec<Json> {
    exchange_with_content("long enough result")
}

fn exchange_with_content(content: &str) -> Vec<Json> {
    vec![
        json(r#"{"role":"system","content":"rules"}"#),
        json(r#"{"role":"user","content":"task"}"#),
        json(
            r#"{"role":"assistant","tool_calls":[{"id":"call_1","function":{"name":"lookup","arguments":"{}"}}]}"#,
        ),
        json(&format!(
            r#"{{"role":"tool","tool_call_id":"call_1","content":"{content}"}}"#
        )),
        json(r#"{"role":"assistant","content":"done"}"#),
    ]
}

#[test]
fn candidate_exchanges_keeps_an_unprotected_complete_exchange() {
    let mut settings = settings();
    settings.min_chars_to_evaluate = MinCharsToEvaluate(5);
    let guardrail = guardrail(settings);
    assert_eq!(
        candidate_exchanges(&guardrail, &completed_exchange()),
        vec![vec![2, 3]]
    );
}

#[test]
fn candidate_exchanges_skips_a_trailing_protected_exchange() {
    let guardrail = guardrail(settings());
    let messages = vec![
        json(r#"{"role":"system","content":"rules"}"#),
        json(r#"{"role":"user","content":"task"}"#),
        json(r#"{"role":"assistant","tool_calls":[{"id":"call_1"}]}"#),
        json(r#"{"role":"tool","tool_call_id":"call_1","content":"result"}"#),
    ];
    assert!(candidate_exchanges(&guardrail, &messages).is_empty());
}

#[test]
fn candidate_exchanges_skips_a_tool_call_without_a_result() {
    let mut settings = settings();
    settings.min_chars_to_evaluate = MinCharsToEvaluate(0);
    let guardrail = guardrail(settings);
    let messages = vec![
        json(r#"{"role":"system","content":"rules"}"#),
        json(r#"{"role":"user","content":"task"}"#),
        json(r#"{"role":"assistant","tool_calls":[{"id":"call_1"}]}"#),
        json(r#"{"role":"assistant","content":"done"}"#),
    ];
    assert!(candidate_exchanges(&guardrail, &messages).is_empty());
}

#[rstest]
#[case::below_minimum("abcd", 5, false)]
#[case::at_minimum("abcde", 5, true)]
#[case::multibyte_below_by_characters("\u{e9}\u{e9}", 3, false)]
#[case::multibyte_at_minimum("\u{e9}\u{e9}\u{e9}", 3, true)]
fn candidate_exchanges_filters_by_tool_text_length(
    #[case] content: &str,
    #[case] min_chars: usize,
    #[case] kept: bool,
) {
    let mut settings = settings();
    settings.min_chars_to_evaluate = MinCharsToEvaluate(min_chars);
    let guardrail = guardrail(settings);
    let candidates = candidate_exchanges(&guardrail, &exchange_with_content(content));
    assert_eq!(candidates == vec![vec![2, 3]], kept);
}

fn unprotected(_messages: &[Json]) -> Vec<usize> {
    Vec::new()
}

fn user_then_tool(_messages: &[Json]) -> Vec<Vec<usize>> {
    vec![vec![1, 2]]
}

#[test]
fn candidate_exchanges_requires_an_assistant_at_the_start_of_the_group() {
    let mut settings = settings();
    settings.min_chars_to_evaluate = MinCharsToEvaluate(0);
    let mut guardrail = guardrail(settings);
    guardrail.get_protected_indices = unprotected;
    guardrail.group_tool_exchanges = user_then_tool;
    let messages = vec![
        json(r#"{"role":"system","content":"rules"}"#),
        json(r#"{"role":"user","content":"task"}"#),
        json(r#"{"role":"tool","content":"result"}"#),
    ];
    assert!(candidate_exchanges(&guardrail, &messages).is_empty());
}

fn many_exchanges(count: usize) -> Vec<Json> {
    let mut messages = vec![
        json(r#"{"role":"system","content":"rules"}"#),
        json(r#"{"role":"user","content":"task"}"#),
    ];
    for ordinal in 0..count {
        messages.push(json(&format!(
            r#"{{"role":"assistant","tool_calls":[{{"id":"call_{ordinal}"}}]}}"#
        )));
        messages.push(json(&format!(
            r#"{{"role":"tool","tool_call_id":"call_{ordinal}","content":"result"}}"#
        )));
    }
    messages.push(json(r#"{"role":"assistant","content":"done"}"#));
    messages
}

#[test]
fn candidate_exchanges_caps_evaluation_to_the_last_two_hundred_exchanges() {
    assert_eq!(MAX_EXCHANGES_EVALUATED, 200);
    let mut settings = settings();
    settings.min_chars_to_evaluate = MinCharsToEvaluate(0);
    let guardrail = guardrail(settings);
    let candidates = candidate_exchanges(&guardrail, &many_exchanges(201));
    assert_eq!(candidates.len(), 200);
    assert_eq!(candidates.first(), Some(&vec![4, 5]));
    assert_eq!(candidates.last(), Some(&vec![402, 403]));
}

#[test]
fn candidate_exchanges_keeps_every_exchange_below_the_cap() {
    let mut settings = settings();
    settings.min_chars_to_evaluate = MinCharsToEvaluate(0);
    let guardrail = guardrail(settings);
    let candidates = candidate_exchanges(&guardrail, &many_exchanges(200));
    assert_eq!(candidates.len(), 200);
    assert_eq!(candidates.first(), Some(&vec![2, 3]));
    assert_eq!(candidates.last(), Some(&vec![400, 401]));
}
