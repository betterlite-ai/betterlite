#![cfg(test)]

use rstest::rstest;

use super::truncate_for_state;
use crate::typesafe::ELISION_MARKER;

#[test]
fn truncate_for_state_keeps_a_short_text() {
    assert_eq!(truncate_for_state("short", 100), "short");
}

#[test]
fn truncate_for_state_keeps_text_exactly_at_the_limit() {
    assert_eq!(truncate_for_state("0123456789", 10), "0123456789");
}

#[test]
fn truncate_for_state_marker_is_the_litellm_elision_text() {
    assert_eq!(ELISION_MARKER, "\n... [middle truncated] ...\n");
    assert_eq!(ELISION_MARKER.chars().count(), 28);
}

#[rstest]
#[case::zero(0, "")]
#[case::one(1, "0")]
#[case::marker_length(28, "0123456789012345678901234567")]
fn truncate_for_state_shortens_when_the_limit_does_not_fit_the_marker(
    #[case] max_chars: usize,
    #[case] expected: &str,
) {
    let text = "0123456789".repeat(4);
    assert_eq!(truncate_for_state(&text, max_chars), expected);
}

#[test]
fn truncate_for_state_keeps_head_and_tail_around_the_marker() {
    let text = "0123456789".repeat(10);
    let truncated = truncate_for_state(&text, 40);
    assert_eq!(truncated, format!("012345{ELISION_MARKER}456789"));
    assert_eq!(truncated.chars().count(), 40);
}

#[test]
fn truncate_for_state_keeps_one_tail_character_when_the_budget_is_one() {
    let text = "0123456789".repeat(4);
    assert_eq!(truncate_for_state(&text, 29), format!("{ELISION_MARKER}9"));
}

#[test]
fn truncate_for_state_counts_characters_not_bytes() {
    let text = "\u{e9}".repeat(50);
    let truncated = truncate_for_state(&text, 40);
    assert_eq!(truncated.chars().count(), 40);
    assert_eq!(
        truncated,
        format!(
            "{}{ELISION_MARKER}{}",
            "\u{e9}".repeat(6),
            "\u{e9}".repeat(6)
        )
    );
}
