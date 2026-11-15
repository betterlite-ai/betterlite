use betterlite_context::agentic_callback::AgenticCall;
use rstest::rstest;

use crate::compresr_recovery::error::RecoveryError;
use crate::compresr_recovery::hash::{content_hash, display_hash, recovery_marker};
use crate::compresr_recovery::plan::should_run;
use crate::compresr_recovery::store::entry_bytes;

use super::support::{RETRIEVE_TOOL, map, messages, originals, raw};

#[rstest]
#[case::empty("", "e3b0c44298fc1c149afbf4c8")]
#[case::ascii(
    "The quick brown fox jumps over the lazy dog",
    "d7a8fbb307d7809469ca9abc"
)]
#[case::multibyte("\u{e9}", "4a99557e4033c3539de2eb65")]
fn content_is_a_24_character_sha256_prefix_hash(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(content_hash(text), expected);
}

#[rstest]
#[case::short("abc123", "abc123")]
#[case::space_stays("a b", "a b")]
#[case::control_characters("ab\ncd\u{1b}[31m", "abcd[31m")]
#[case::separators("a\u{a0}\u{2028}\u{2029}\u{3000}b", "ab")]
#[case::format_character("a\u{200b}b", "ab")]
#[case::private_use("a\u{e000}b", "ab")]
#[case::unassigned("a\u{378}b", "ab")]
fn display_text_keeps_printable_characters_only_hash(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(display_hash(input), expected);
}

#[rstest]
#[case::at_the_limit(32, false)]
#[case::over_the_limit(33, true)]
#[case::far_over(500, true)]
fn display_text_is_cut_past_32_characters_hash(#[case] count: usize, #[case] cut: bool) {
    let expected = if cut {
        format!("{}\u{2026}", "a".repeat(32))
    } else {
        "a".repeat(count)
    };
    assert_eq!(display_hash(&"a".repeat(count)), expected);
}

#[test]
fn display_text_counts_characters_not_bytes_hash() {
    assert_eq!(display_hash(&"\u{e9}".repeat(32)), "\u{e9}".repeat(32));
}

#[test]
fn display_text_strips_before_it_cuts_hash() {
    let input = format!("{}\n{}", "a".repeat(20), "b".repeat(20));
    let expected = format!("{}{}\u{2026}", "a".repeat(20), "b".repeat(12));
    assert_eq!(display_hash(&input), expected);
}

#[test]
fn recovery_marker_names_the_hash_and_the_tool_hash() {
    assert_eq!(
        recovery_marker("abc"),
        "\n\n[compresr hash=abc: parts of this content were compressed away. If you need the full original, call the compresr_retrieve tool with this hash.]"
    );
}

#[test]
fn entry_bytes_counts_utf8_bytes_of_every_original_hash() {
    assert_eq!(entry_bytes(&originals(&[("a", "\u{e9}"), ("b", "xyz")])), 5);
}

// Python hashes a lone \ud800 escape with surrogatepass; betterlite's response parse refuses it first, so none reaches the hash
#[test]
fn a_lone_surrogate_escape_is_rejected_before_hashing_hash() {
    let response = raw(r#"{"choices":[{"message":{"content":"\ud800"}}]}"#);
    let tools = messages(RETRIEVE_TOOL);
    let kwargs = map("{}");
    let call = AgenticCall {
        response: &response,
        model: "gpt-4o",
        messages: &[],
        tools: Some(&tools),
        stream: false,
        custom_llm_provider: "openai",
        kwargs: &kwargs,
    };
    assert!(matches!(
        should_run(&call),
        Err(RecoveryError::Response { .. })
    ));
}
