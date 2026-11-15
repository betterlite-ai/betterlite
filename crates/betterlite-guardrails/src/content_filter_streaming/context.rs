use crate::content_filter_streaming::plan::StreamedScanPlan;
use crate::content_filter_streaming::text::char_slice;

mod tests;

#[must_use]
pub fn cut_breaks_wider_context(
    buffered_text: &str,
    head: &str,
    tail: &str,
    plan: &StreamedScanPlan,
) -> bool {
    let tail_lower = tail.to_lowercase();
    let phrase_leaves = exception_phrases_in(buffered_text, plan)
        .iter()
        .any(|phrase| !tail_lower.contains(phrase.as_str()));
    phrase_leaves || !conditional_words_at_cut(head, tail, plan).is_empty()
}

// The exception phrases Python's unchanged buffer holds, which suppress every category match
#[must_use]
pub fn exception_phrases_in(text: &str, plan: &StreamedScanPlan) -> Vec<String> {
    let text_lower = text.to_lowercase();
    plan.exception_phrases
        .iter()
        .filter(|phrase| text_lower.contains(phrase.as_str()))
        .cloned()
        .collect()
}

// The conditional words in the sentence the cut splits, as re.split gives its two halves
#[must_use]
pub fn conditional_words_at_cut(head: &str, tail: &str, plan: &StreamedScanPlan) -> Vec<String> {
    let head_lower = head.to_lowercase();
    let tail_lower = tail.to_lowercase();
    let cut_head = head_lower
        .split(['.', '!', '?'])
        .next_back()
        .unwrap_or_default();
    let cut_tail = tail_lower.split(['.', '!', '?']).next().unwrap_or_default();
    let cut_sentence = format!("{cut_head}{cut_tail}");
    plan.conditional_words
        .iter()
        .filter(|word| cut_sentence.contains(word.as_str()))
        .cloned()
        .collect()
}

// The dropped part of the sentence the cut splits, as text so the next scan keeps its spacing
#[must_use]
pub fn open_sentence_prefix(previous: &str, head: &str, context_chars: usize) -> String {
    let open = if head.contains(['.', '!', '?']) {
        head.split(['.', '!', '?'])
            .next_back()
            .unwrap_or_default()
            .to_owned()
    } else {
        format!("{previous}{head}")
    };
    // a sentence longer than two contexts keeps its end, the part nearest the buffer
    let limit = context_chars.saturating_mul(2);
    let length = open.chars().count();
    let start = word_end_bound(&open, length.saturating_sub(limit));
    char_slice(&open, start, length).to_owned()
}

// A bound that splits a word moves to the next non-word character, or stays when the sentence ends inside that word
fn word_end_bound(open: &str, start: usize) -> usize {
    if start == 0 {
        return 0;
    }
    let before = open.chars().nth(start.saturating_sub(1));
    let at = open.chars().nth(start);
    let inside = matches!(
        (before, at),
        (Some(left), Some(right))
            if (left.is_alphanumeric() || left == '_') && (right.is_alphanumeric() || right == '_')
    );
    if !inside {
        return start;
    }
    open.chars()
        .skip(start)
        .position(|character| !(character.is_alphanumeric() || character == '_'))
        .map_or(start, |offset| start.saturating_add(offset))
}
