#![cfg(test)]

use crate::content_filter_streaming::context::cut_breaks_wider_context;
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
