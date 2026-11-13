#![cfg(test)]

use super::estimate::estimate_tokens_for_request;
use super::support::{estimate, no_translation, request};

fn total(body: &str, call_type: Option<&str>, min_limit: Option<i64>) -> i64 {
    let data = request(body);
    estimate_tokens_for_request(
        &no_translation(),
        &estimate(&data, call_type, min_limit, None),
    )
}

#[test]
fn the_estimate_adds_the_input_and_the_output_budget() {
    assert_eq!(
        total(
            r#"{"messages":[{"role":"user","content":"abcdefgh"}],"max_tokens":10}"#,
            Some("completion"),
            None
        ),
        12
    );
}

#[test]
fn the_output_budget_multiplies_with_the_candidate_count() {
    assert_eq!(
        total(
            r#"{"messages":[{"role":"user","content":"abcd"}],"max_tokens":10,"n":2}"#,
            Some("completion"),
            None
        ),
        21
    );
}

#[test]
fn a_zero_cap_with_no_input_reserves_nothing() {
    assert_eq!(
        total(
            r#"{"messages":[],"max_tokens":0}"#,
            Some("completion"),
            None
        ),
        0
    );
}

#[test]
fn a_zero_cap_with_input_reserves_only_the_input() {
    assert_eq!(
        total(
            r#"{"messages":[{"role":"user","content":"abcdefgh"}],"max_tokens":0}"#,
            Some("completion"),
            None
        ),
        2
    );
}

#[test]
fn the_floor_joins_the_input_when_no_cap_is_configured() {
    assert_eq!(
        total(r#"{"input":"abcdefgh"}"#, Some("responses"), Some(400)),
        102
    );
}
