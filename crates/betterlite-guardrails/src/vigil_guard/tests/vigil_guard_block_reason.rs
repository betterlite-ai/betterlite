use betterlite_guardrails::vigil_guard::block_reason::build_block_reason;

use crate::support::analysis;

#[test]
fn block_message_is_trimmed() {
    let analysis = analysis(r#"{"blockMessage":"  banned content  "}"#);
    assert_eq!(build_block_reason(&analysis), "banned content");
}

#[test]
fn block_message_is_truncated_to_500_characters() {
    let message = "x".repeat(600);
    let analysis = analysis(&format!(r#"{{"blockMessage":"{message}"}}"#));
    assert_eq!(build_block_reason(&analysis), "x".repeat(500));
}

#[test]
fn decision_reason_is_used_when_the_block_message_is_absent() {
    let analysis = analysis(r#"{"decisionReason":"policy"}"#);
    assert_eq!(build_block_reason(&analysis), "policy");
}

#[test]
fn a_blank_block_message_falls_through_to_the_decision_reason() {
    let analysis = analysis(r#"{"blockMessage":"   ","decisionReason":"policy"}"#);
    assert_eq!(build_block_reason(&analysis), "policy");
}

#[test]
fn a_non_string_block_message_falls_through() {
    let analysis = analysis(r#"{"blockMessage":3,"decisionReason":"policy"}"#);
    assert_eq!(build_block_reason(&analysis), "policy");
}

#[test]
fn categories_are_joined_with_commas() {
    let analysis = analysis(r#"{"categories":["toxicity","pii"]}"#);
    assert_eq!(build_block_reason(&analysis), "toxicity, pii");
}

#[test]
fn blank_and_non_string_categories_are_skipped() {
    let analysis = analysis(r#"{"categories":["toxicity"," ",7,"pii"]}"#);
    assert_eq!(build_block_reason(&analysis), "toxicity, pii");
}

#[test]
fn truncation_happens_after_joining() {
    let first = "x".repeat(300);
    let second = "y".repeat(300);
    let analysis = analysis(&format!(r#"{{"categories":["{first}","{second}"]}}"#));
    let expected = format!("{}, {}", first, "y".repeat(198));
    assert_eq!(build_block_reason(&analysis), expected);
}

#[test]
fn no_candidates_use_the_default_reason() {
    let analysis = analysis("{}");
    assert_eq!(build_block_reason(&analysis), "Blocked by policy");
}

#[test]
fn the_block_message_wins_over_categories() {
    let analysis = analysis(r#"{"blockMessage":"banned","categories":["toxicity"]}"#);
    assert_eq!(build_block_reason(&analysis), "banned");
}

#[test]
fn non_array_categories_are_ignored() {
    let analysis = analysis(r#"{"categories":"toxicity"}"#);
    assert_eq!(build_block_reason(&analysis), "Blocked by policy");
}
