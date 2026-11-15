use rstest::rstest;

use crate::api::vertex_chirp_protocol::words::{join_transcript, new_words};

#[rstest]
#[case::shared_prefix_with_punctuation("Hello world", "hello, world!", "")]
#[case::case_change_inside_the_prefix("hello world", "Hello World", "")]
#[case::apostrophe_inside_the_prefix("don't", "dont", "")]
#[case::full_case_fold("Stra\u{df}e", "STRASSE", "")]
#[case::appended_words("hello", "hello there", " there")]
#[case::changed_first_word("hello", "yellow", "yellow")]
#[case::nothing_new("hello there", "hello", "")]
#[case::empty_current("hello", "", "")]
#[case::several_words("a b", "a b c d", " c d")]
#[case::punctuation_between_words("um", "um, hello", " hello")]
#[case::accents_are_kept("caf\u{e9}", "cafe", "cafe")]
#[case::vowel_signs_are_kept(
    "\u{939}\u{93f}\u{902}\u{926}\u{940} \u{915}\u{93e}",
    "\u{939}\u{93f}\u{902}\u{926}\u{940} \u{915}\u{940}",
    " \u{915}\u{940}"
)]
fn emits_only_new_words(#[case] previous: &str, #[case] current: &str, #[case] expected: &str) {
    assert_eq!(new_words(previous, current), expected);
}

#[rstest]
#[case::both("hello", "there", "hello there")]
#[case::committed_only("hello", "", "hello")]
#[case::tail_only("", "there", "there")]
#[case::neither("", "", "")]
fn joins_transcripts(#[case] committed: &str, #[case] tail: &str, #[case] expected: &str) {
    assert_eq!(join_transcript(committed, tail), expected);
}
