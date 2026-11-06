use crate::spend_sanitize::truncate::{TRUNCATION_DB_SAFEGUARD_NOTE, truncate_text};

fn marker(skipped: i64) -> String {
    format!("... (litellm_truncated skipped {skipped} chars. {TRUNCATION_DB_SAFEGUARD_NOTE}) ...")
}

#[test]
fn keeps_a_thirty_five_percent_head_and_a_sixty_five_percent_tail() {
    let expected = format!("a{}ij", marker(7));
    assert_eq!(truncate_text("abcdefghij", 4), expected);
}

#[test]
fn leaves_text_at_or_below_the_limit_untouched() {
    assert_eq!(truncate_text("abcdefghij", 10), "abcdefghij");
    assert_eq!(truncate_text("", 0), "");
}

#[test]
fn keeps_the_whole_tail_when_the_kept_tail_is_zero() {
    let expected = format!("{}abcdefghij", marker(10));
    assert_eq!(truncate_text("abcdefghij", 0), expected);
    assert_eq!(truncate_text("abcdefghij", 1), expected);
}

#[test]
fn counts_back_from_both_ends_for_a_negative_limit() {
    let expected = format!("abcdefg{}hij", marker(19));
    assert_eq!(truncate_text("abcdefghij", -10), expected);
    let single = format!("{}bcdefghij", marker(10));
    assert_eq!(truncate_text("abcdefghij", -1), single);
}

#[test]
fn counts_unicode_characters_rather_than_bytes() {
    let expected = format!("a{}j\u{e9}", marker(8));
    assert_eq!(truncate_text("abcdefghij\u{e9}", 4), expected);
    let accented = format!("\u{e9}{}\u{e9}\u{e9}", marker(3));
    assert_eq!(
        truncate_text("\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}", 4),
        accented
    );
}

// The exact ratio keeps 63 head and 117 tail characters where Python's float split keeps 62 and 117
#[test]
fn uses_exact_ratios_at_the_180_character_boundary() {
    let text = "x".repeat(200);
    let truncated = truncate_text(&text, 180);
    let expected = format!("{}{}x", "x".repeat(63), marker(20));
    assert!(truncated.starts_with(&expected), "{truncated}");
    assert!(truncated.ends_with(&"x".repeat(117)));
    assert!(!truncated.contains("skipped 21 chars"));
}
