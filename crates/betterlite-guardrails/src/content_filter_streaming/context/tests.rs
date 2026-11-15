#![cfg(test)]

use crate::content_filter_streaming::context::{
    conditional_words_at_cut, cut_breaks_wider_context, exception_phrases_in, open_sentence_prefix,
};
use crate::content_filter_streaming::plan::StreamedScanPlan;

fn plan(exceptions: &[&str], conditionals: &[&str]) -> StreamedScanPlan {
    StreamedScanPlan {
        context_chars: 512,
        exception_phrases: exceptions
            .iter()
            .map(|phrase| (*phrase).to_owned())
            .collect(),
        conditional_words: conditionals.iter().map(|word| (*word).to_owned()).collect(),
    }
}

fn head_tail(head: String, tail: String) -> (String, String, String) {
    let buffered = format!("{head}{tail}");
    (buffered, head, tail)
}

#[test]
fn exception_phrase_in_the_head_alone_breaks_the_context() {
    let plan = plan(&["allowed phrase"], &[]);
    let (buffered, head, tail) = head_tail(
        format!("allowed phrase{}", "x".repeat(488)),
        "x".repeat(512),
    );
    assert!(cut_breaks_wider_context(&buffered, &head, &tail, &plan));
}

#[test]
fn exception_phrase_kept_in_the_tail_does_not_break_the_context() {
    let plan = plan(&["allowed phrase"], &[]);
    let (buffered, head, tail) = head_tail(
        "x".repeat(578),
        format!("{}allowed phrase", "x".repeat(498)),
    );
    assert!(!cut_breaks_wider_context(&buffered, &head, &tail, &plan));
}

#[test]
fn exception_phrase_matching_ignores_case() {
    let plan = plan(&["allowed phrase"], &[]);
    let (buffered, head, tail) = head_tail(
        format!("ALLOWED PHRASE{}", "x".repeat(488)),
        "x".repeat(512),
    );
    assert!(cut_breaks_wider_context(&buffered, &head, &tail, &plan));
}

#[test]
fn conditional_word_in_the_cut_sentence_breaks_the_context() {
    let plan = plan(&[], &["child", "harm"]);
    let (buffered, head, tail) = head_tail(format!("{}child", "x".repeat(580)), "y".repeat(516));
    assert!(cut_breaks_wider_context(&buffered, &head, &tail, &plan));
}

#[test]
fn conditional_word_split_across_the_cut_breaks_the_context() {
    let plan = plan(&[], &["child"]);
    let (buffered, head, tail) = head_tail(
        format!("{}chi", "x".repeat(580)),
        format!("ld{}", "y".repeat(516)),
    );
    assert!(cut_breaks_wider_context(&buffered, &head, &tail, &plan));
}

#[test]
fn conditional_word_in_the_first_tail_sentence_breaks_the_context() {
    let plan = plan(&[], &["child"]);
    let (buffered, head, tail) = head_tail("xxharm".to_owned(), " child. more".to_owned());
    assert!(cut_breaks_wider_context(&buffered, &head, &tail, &plan));
}

#[test]
fn conditional_word_away_from_the_cut_does_not_break_the_context() {
    let plan = plan(&[], &["child"]);
    let (buffered, head, tail) = head_tail(format!("child. {}", "x".repeat(582)), "y".repeat(516));
    assert!(!cut_breaks_wider_context(&buffered, &head, &tail, &plan));
}

#[test]
fn conditional_word_breaking_a_sentence_boundary_does_not_break_the_context() {
    let plan = plan(&[], &["harm"]);
    let (buffered, head, tail) = head_tail(
        format!("{}harm. ch", "x".repeat(580)),
        format!("ild{}", "y".repeat(516)),
    );
    assert!(!cut_breaks_wider_context(&buffered, &head, &tail, &plan));
}

#[test]
fn conditional_word_before_a_trailing_terminator_does_not_break_the_context() {
    let plan = plan(&[], &["harm"]);
    let (buffered, head, tail) = head_tail("harm.".to_owned(), " child text".to_owned());
    assert!(!cut_breaks_wider_context(&buffered, &head, &tail, &plan));
}

#[test]
fn plain_text_without_phrases_or_words_never_breaks_the_context() {
    let plan = plan(&[], &[]);
    let (buffered, head, tail) = head_tail("x".repeat(588), "y".repeat(516));
    assert!(!cut_breaks_wider_context(&buffered, &head, &tail, &plan));
}

#[test]
fn exception_phrases_in_reports_the_phrases_the_text_holds() {
    let plan = plan(&["allowed phrase", "other phrase"], &[]);
    assert_eq!(
        exception_phrases_in("ALLOWED PHRASE inside", &plan),
        vec!["allowed phrase".to_owned()]
    );
}

#[test]
fn exception_phrases_in_reports_nothing_for_plain_text() {
    let plan = plan(&["allowed phrase"], &[]);
    assert!(exception_phrases_in("plain text", &plan).is_empty());
}

#[test]
fn conditional_words_at_cut_reports_the_words_in_the_joined_sentence() {
    let plan = plan(&[], &["child", "harm"]);
    let head = format!("{}child", "x".repeat(580));
    let tail = "y".repeat(516);
    assert_eq!(
        conditional_words_at_cut(&head, &tail, &plan),
        vec!["child".to_owned()]
    );
}

#[test]
fn conditional_words_at_cut_reports_a_word_split_across_the_cut() {
    let plan = plan(&[], &["child"]);
    let head = format!("{}chi", "x".repeat(580));
    let tail = format!("ld{}", "y".repeat(516));
    assert_eq!(
        conditional_words_at_cut(&head, &tail, &plan),
        vec!["child".to_owned()]
    );
}

#[test]
fn conditional_words_at_cut_skips_a_word_away_from_the_cut() {
    let plan = plan(&[], &["child"]);
    let head = format!("child. {}", "x".repeat(582));
    let tail = "y".repeat(516);
    assert!(conditional_words_at_cut(&head, &tail, &plan).is_empty());
}

#[test]
fn open_sentence_prefix_joins_the_previous_sentence_to_the_head() {
    assert_eq!(
        open_sentence_prefix("child ", "harm rest", 512),
        "child harm rest"
    );
}

#[test]
fn open_sentence_prefix_keeps_only_the_heads_last_sentence() {
    assert_eq!(open_sentence_prefix("gone ", "aa. harm", 512), " harm");
}

#[test]
fn open_sentence_prefix_starts_a_new_sentence_at_any_terminator() {
    assert_eq!(open_sentence_prefix("gone ", "x! harm", 512), " harm");
    assert_eq!(open_sentence_prefix("gone ", "x? harm", 512), " harm");
}

#[test]
fn open_sentence_prefix_ends_the_sentence_at_a_trailing_terminator() {
    assert!(open_sentence_prefix("child ", "xx harm.", 512).is_empty());
}

#[test]
fn open_sentence_prefix_keeps_a_sentence_that_fits_two_contexts() {
    assert_eq!(open_sentence_prefix("", "01234567", 4), "01234567");
}

#[test]
fn open_sentence_prefix_keeps_the_end_of_a_sentence_over_two_contexts() {
    assert_eq!(open_sentence_prefix("", "0123456789", 4), "23456789");
}

#[test]
fn open_sentence_prefix_moves_a_bound_that_splits_a_word() {
    // the bound would land inside "zzzzzzzaharm", so the prefix starts at the next space instead
    assert_eq!(open_sentence_prefix("", "zzzzzzzaharm ch", 4), " ch");
}
