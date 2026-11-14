use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_filter_records::ContentFilterDetection;

use crate::content_filter_actions::{ContentFilterSource, filter_single_text};
use crate::content_filter_streaming::context::{conditional_words_at_cut, exception_phrases_in};
use crate::content_filter_streaming::plan::StreamedScanPlan;
use crate::content_filter_streaming::state::StreamedChoiceState;
use crate::content_filter_streaming::text::char_boundary;

mod tests;

#[must_use]
pub fn trim_streamed_choice_buffer(
    source: &ContentFilterSource<'_>,
    state: StreamedChoiceState,
    masked_text: &str,
    plan: &StreamedScanPlan,
) -> StreamedChoiceState {
    let buffered_len = state.buffered_text.chars().count();
    if buffered_len
        <= plan
            .context_chars
            .saturating_mul(2)
            .max(state.next_trim_len)
    {
        return state;
    }
    let boundary = char_boundary(
        &state.buffered_text,
        buffered_len.saturating_sub(plan.context_chars),
    );
    let head = state.buffered_text.get(..boundary).unwrap_or_default();
    let tail = state.buffered_text.get(boundary..).unwrap_or_default();
    // Python's buffer would keep every phrase and open sentence it holds, and its scan suppresses
    // every category match for them, so the dropped prefix travels as flags instead of text
    let dropped_phrases = exception_phrases_in(&state.buffered_text, plan);
    let suppression = !state.seen_exception_phrases.is_empty() || !dropped_phrases.is_empty();
    let cut = committable_cut(source, &state, (head, tail), masked_text, suppression);
    let cut_words = conditional_words_at_cut(head, tail, plan);
    let tail_text = tail.to_owned();
    let next_trim_len = buffered_len.saturating_add(plan.context_chars);
    match cut {
        None => deferred(state, next_trim_len),
        Some((head_detections, masked_head_len)) => trimmed(
            state,
            tail_text,
            &head_detections,
            masked_head_len,
            dropped_phrases,
            cut_words,
        ),
    }
}

// The buffer stays whole and only the bar for the next attempt moves, one context past its length
fn deferred(state: StreamedChoiceState, next_trim_len: usize) -> StreamedChoiceState {
    StreamedChoiceState {
        next_trim_len,
        ..state
    }
}

// The dropped prefix leaves the buffer, its detections become committed, and the yielded count drops by it
fn trimmed(
    state: StreamedChoiceState,
    tail: String,
    head_detections: &[ContentFilterDetection],
    masked_head_len: usize,
    dropped_phrases: Vec<String>,
    cut_words: Vec<String>,
) -> StreamedChoiceState {
    let mut seen_exception_phrases = state.seen_exception_phrases;
    for phrase in dropped_phrases {
        if !seen_exception_phrases.contains(&phrase) {
            seen_exception_phrases.push(phrase);
        }
    }
    let mut open_sentence_words = state.open_sentence_words;
    for word in cut_words {
        if !open_sentence_words.contains(&word) {
            open_sentence_words.push(word);
        }
    }
    StreamedChoiceState {
        buffered_text: tail,
        yielded_masked_text_len: state
            .yielded_masked_text_len
            .saturating_sub(masked_head_len),
        committed_detections: state
            .committed_detections
            .iter()
            .chain(head_detections)
            .cloned()
            .collect(),
        latest_detections: state.latest_detections,
        next_trim_len: 0,
        seen_exception_phrases,
        open_sentence_words,
    }
}

fn committable_cut(
    source: &ContentFilterSource<'_>,
    state: &StreamedChoiceState,
    halves: (&str, &str),
    masked_text: &str,
    suppression: bool,
) -> Option<(Vec<ContentFilterDetection>, usize)> {
    let (head, tail) = halves;
    let suppressed = CategoryLoadResult::default();
    let suppressed_source = ContentFilterSource {
        categories: &suppressed,
        patterns: source.patterns,
        blocked_words: source.blocked_words,
        pattern_redaction_format: source.pattern_redaction_format,
        keyword_redaction_tag: source.keyword_redaction_tag,
    };
    let scan_source: &ContentFilterSource<'_> = if suppression {
        &suppressed_source
    } else {
        source
    };
    let mut head_detections = Vec::new();
    let masked_head = filter_single_text(scan_source, head, Some(&mut head_detections)).ok()?;
    let masked_tail = filter_single_text(scan_source, tail, None).ok()?;
    let masked_head_len = masked_head.chars().count();
    let mut combined = masked_head;
    combined.push_str(&masked_tail);
    if combined != masked_text || masked_head_len > state.yielded_masked_text_len {
        return None;
    }
    Some((head_detections, masked_head_len))
}
