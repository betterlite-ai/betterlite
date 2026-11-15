use std::collections::BTreeMap;

use reqwest::StatusCode;

use super::support::{analysis, guardrail, params, threshold_map, violation};
use crate::azure_text_moderation::config::DEFAULT_SEVERITY_THRESHOLD;

#[test]
fn blocks_at_the_default_threshold() {
    let hook = guardrail("https://example.test", params(None, None));
    let error = hook
        .check_severity_threshold(&analysis(&[("Hate", 2)]))
        .unwrap_err();
    assert_eq!(violation(error), ("Hate".to_owned(), 2, 2));
    assert_eq!(DEFAULT_SEVERITY_THRESHOLD, 2);
}

#[test]
fn passes_below_the_default_threshold() {
    let hook = guardrail("https://example.test", params(None, None));
    hook.check_severity_threshold(&analysis(&[("Hate", 1)]))
        .unwrap();
}

#[test]
fn reports_the_first_category_at_or_above_the_default() {
    let hook = guardrail("https://example.test", params(None, None));
    let error = hook
        .check_severity_threshold(&analysis(&[("Sexual", 1), ("Hate", 4), ("Violence", 5)]))
        .unwrap_err();
    assert_eq!(violation(error), ("Hate".to_owned(), 2, 4));
}

#[test]
fn blocks_at_the_global_threshold() {
    let hook = guardrail("https://example.test", params(Some(3), None));
    let error = hook
        .check_severity_threshold(&analysis(&[("Hate", 3)]))
        .unwrap_err();
    assert_eq!(violation(error), ("Hate".to_owned(), 3, 3));
    hook.check_severity_threshold(&analysis(&[("Hate", 2)]))
        .unwrap();
}

#[test]
fn the_global_threshold_suppresses_the_default() {
    let hook = guardrail("https://example.test", params(Some(4), None));
    hook.check_severity_threshold(&analysis(&[("Hate", 3)]))
        .unwrap();
}

#[test]
fn an_empty_category_map_suppresses_the_default() {
    let hook = guardrail("https://example.test", params(None, Some(BTreeMap::new())));
    hook.check_severity_threshold(&analysis(&[("Violence", 3)]))
        .unwrap();
}

#[test]
fn an_empty_category_map_leaves_the_global_threshold_on() {
    let hook = guardrail(
        "https://example.test",
        params(Some(2), Some(BTreeMap::new())),
    );
    let error = hook
        .check_severity_threshold(&analysis(&[("Hate", 2)]))
        .unwrap_err();
    assert_eq!(violation(error), ("Hate".to_owned(), 2, 2));
}

#[test]
fn checks_the_category_threshold_before_the_global_one() {
    let hook = guardrail(
        "https://example.test",
        params(Some(4), Some(threshold_map(&[("Hate", 1)]))),
    );
    let error = hook
        .check_severity_threshold(&analysis(&[("Hate", 2)]))
        .unwrap_err();
    assert_eq!(violation(error), ("Hate".to_owned(), 1, 2));
}

#[test]
fn blocks_at_a_category_threshold_and_passes_below_it() {
    let hook = guardrail(
        "https://example.test",
        params(None, Some(threshold_map(&[("Hate", 3)]))),
    );
    let error = hook
        .check_severity_threshold(&analysis(&[("Hate", 3)]))
        .unwrap_err();
    assert_eq!(violation(error), ("Hate".to_owned(), 3, 3));
    hook.check_severity_threshold(&analysis(&[("Hate", 2)]))
        .unwrap();
}

#[test]
fn a_category_threshold_covers_only_its_category() {
    let hook = guardrail(
        "https://example.test",
        params(None, Some(threshold_map(&[("Hate", 1)]))),
    );
    hook.check_severity_threshold(&analysis(&[("Violence", 3)]))
        .unwrap();
}

#[test]
fn ignores_a_response_category_outside_the_four() {
    let hook = guardrail(
        "https://example.test",
        params(None, Some(threshold_map(&[("Hate", 1)]))),
    );
    hook.check_severity_threshold(&analysis(&[("Profanity", 7)]))
        .unwrap();
}

#[test]
fn an_explicit_zero_global_threshold_blocks_severity_zero() {
    let hook = guardrail("https://example.test", params(Some(0), None));
    let error = hook
        .check_severity_threshold(&analysis(&[("Hate", 0)]))
        .unwrap_err();
    assert_eq!(violation(error), ("Hate".to_owned(), 0, 0));
}

#[test]
fn an_explicit_zero_category_threshold_blocks_severity_zero() {
    let hook = guardrail(
        "https://example.test",
        params(None, Some(threshold_map(&[("Hate", 0)]))),
    );
    let error = hook
        .check_severity_threshold(&analysis(&[("Hate", 0)]))
        .unwrap_err();
    assert_eq!(violation(error), ("Hate".to_owned(), 0, 0));
}

#[test]
fn passes_an_empty_response() {
    let hook = guardrail("https://example.test", params(None, None));
    hook.check_severity_threshold(&analysis(&[])).unwrap();
}

#[test]
fn writes_the_violation_message_like_litellm() {
    let hook = guardrail("https://example.test", params(None, None));
    let error = hook
        .check_severity_threshold(&analysis(&[("Hate", 3)]))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Azure Content Safety Guardrail: Hate crossed severity 2, Got severity: 3"
    );
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
}
