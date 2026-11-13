#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_prompts::anthropic_history::groups::group_tool_exchanges;
use betterlite_prompts::arguments::json::Json;

use super::{compression_protected_indices, protected_indices};
use crate::typesafe::fixtures::json;

fn trailing_exchange() -> Vec<Json> {
    vec![
        json(r#"{"role":"system","content":"rules"}"#),
        json(r#"{"role":"user","content":"question"}"#),
        json(
            r#"{"role":"assistant","tool_calls":[{"id":"call_1","function":{"name":"lookup","arguments":"{}"}}]}"#,
        ),
        json(r#"{"role":"tool","tool_call_id":"call_1","content":"result"}"#),
    ]
}

#[test]
fn protected_indices_expands_a_protected_exchange_over_its_tool_rows() {
    let messages = trailing_exchange();
    assert_eq!(compression_protected_indices(&messages), vec![0, 1, 2]);
    assert_eq!(
        group_tool_exchanges(&messages),
        vec![vec![0], vec![1], vec![2, 3]]
    );
    assert_eq!(
        protected_indices(
            &messages,
            compression_protected_indices,
            group_tool_exchanges
        ),
        BTreeSet::from([0, 1, 2, 3])
    );
}

#[test]
fn protected_indices_keeps_an_unprotected_completed_exchange_out_of_the_set() {
    let messages = vec![
        json(r#"{"role":"system","content":"rules"}"#),
        json(r#"{"role":"user","content":"question"}"#),
        json(
            r#"{"role":"assistant","tool_calls":[{"id":"call_1","function":{"name":"lookup","arguments":"{}"}}]}"#,
        ),
        json(r#"{"role":"tool","tool_call_id":"call_1","content":"result"}"#),
        json(r#"{"role":"assistant","content":"done"}"#),
    ];
    assert_eq!(compression_protected_indices(&messages), vec![0, 1, 4]);
    assert_eq!(
        protected_indices(
            &messages,
            compression_protected_indices,
            group_tool_exchanges
        ),
        BTreeSet::from([0, 1, 4])
    );
}

#[test]
fn protected_indices_keeps_the_prompt_cache_prefix_and_expands_it() {
    let messages = vec![
        json(r#"{"role":"system","content":"rules"}"#),
        json(r#"{"role":"user","content":"question"}"#),
        json(r#"{"role":"assistant","tool_calls":[{"id":"call_1"},{"id":"call_2"}]}"#),
        json(
            r#"{"role":"tool","tool_call_id":"call_1","content":[{"type":"text","text":"first","cache_control":{"type":"ephemeral"}}]}"#,
        ),
        json(r#"{"role":"tool","tool_call_id":"call_2","content":"second"}"#),
        json(r#"{"role":"user","content":"follow up"}"#),
        json(r#"{"role":"assistant","content":"done"}"#),
    ];
    assert_eq!(
        compression_protected_indices(&messages),
        vec![0, 1, 2, 3, 5, 6]
    );
    assert_eq!(
        protected_indices(
            &messages,
            compression_protected_indices,
            group_tool_exchanges
        ),
        BTreeSet::from([0, 1, 2, 3, 4, 5, 6])
    );
}

fn shared_indices(_messages: &[Json]) -> Vec<usize> {
    vec![1]
}

fn duplicated_indices(_messages: &[Json]) -> Vec<usize> {
    vec![1, 1, 2, 2]
}

fn overlapping_groups(_messages: &[Json]) -> Vec<Vec<usize>> {
    vec![vec![1, 2], vec![2, 3]]
}

#[test]
fn protected_indices_expands_from_the_originally_protected_set() {
    let protected = protected_indices(&[json("{}")], shared_indices, overlapping_groups);
    assert_eq!(protected, BTreeSet::from([1, 2]));
}

#[test]
fn protected_indices_deduplicates_repeated_indices() {
    let protected = protected_indices(&[json("{}")], duplicated_indices, overlapping_groups);
    assert_eq!(protected, BTreeSet::from([1, 2, 3]));
}
