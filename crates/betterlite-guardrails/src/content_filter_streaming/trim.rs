use betterlite_config::sections::content_category::loading::CategoryLoadResult;

use crate::content_filter_actions::{ContentFilterSource, filter_single_text};
use crate::content_filter_streaming::context::{
    cut_breaks_wider_context, exception_phrases_in, open_sentence_prefix,
};
use crate::content_filter_streaming::filtering::{
    ScanInput, earlier_match, filter_streamed_text, source_with_categories,
};
use crate::content_filter_streaming::plan::StreamedScanPlan;
use crate::content_filter_streaming::state::{
    CarriedMatches, DetectionCheck, StreamedChoiceState, StreamedDetection,
};
use crate::content_filter_streaming::text::char_boundary;

mod tests;

// What the dropped prefix gave the next scans: its detections and its first match of every check
struct HeadScan {
    detections: Vec<StreamedDetection>,
    masked_len: usize,
    matches: CarriedMatches,
}

// Everything a committed cut hands the state it leaves behind
struct CutResult {
    tail: String,
    head_scan: HeadScan,
    dropped_phrases: Vec<String>,
    open_sentence: String,
    deferring: bool,
}

#[must_use]
pub fn trim_streamed_choice_buffer(
    source: &ContentFilterSource<'_>,
    state: StreamedChoiceState,
    masked_text: &str,
    plan: &StreamedScanPlan,
) -> StreamedChoiceState {
    let buffered_len = state.buffered_text.chars().count();
    let bar = plan
        .context_chars
        .saturating_mul(2)
        .max(state.next_trim_len);
    if buffered_len <= bar {
        return state;
    }
    let boundary = char_boundary(
        &state.buffered_text,
        buffered_len.saturating_sub(plan.context_chars),
    );
    let head = state.buffered_text.get(..boundary).unwrap_or_default();
    let tail = state.buffered_text.get(boundary..).unwrap_or_default();
    // Python's buffer keeps every phrase and open sentence and its scan suppresses their category matches, so the dropped prefix travels as flags and text instead
    let dropped_phrases = exception_phrases_in(&state.buffered_text, plan);
    let suppression = !state.seen_exception_phrases.is_empty() || !dropped_phrases.is_empty();
    let deferring = litellm_defers(&state, head, tail, plan);
    let cut = committable_cut(source, &state, (head, tail), masked_text, suppression);
    let open_sentence = open_sentence_prefix(&state.open_sentence, head, plan.context_chars);
    let next_trim_len = buffered_len.saturating_add(plan.context_chars);
    match cut {
        None => deferred(state, next_trim_len),
        Some(head_scan) => {
            let owned_tail = tail.to_owned();
            trimmed(
                source,
                state,
                CutResult {
                    tail: owned_tail,
                    head_scan,
                    dropped_phrases,
                    open_sentence,
                    deferring,
                },
            )
        }
    }
}

// The carried open sentence stands in for the part of LiteLLM's buffer an earlier trim already dropped
fn litellm_defers(
    state: &StreamedChoiceState,
    head: &str,
    tail: &str,
    plan: &StreamedScanPlan,
) -> bool {
    let carried_head = format!("{}{head}", state.open_sentence);
    let carried_text = format!("{}{}", state.open_sentence, state.buffered_text);
    cut_breaks_wider_context(&carried_text, &carried_head, tail, plan)
}

// The buffer stays whole and only the bar for the next attempt moves, one context past its length
fn deferred(state: StreamedChoiceState, next_trim_len: usize) -> StreamedChoiceState {
    StreamedChoiceState {
        next_trim_len,
        ..state
    }
}

// The dropped prefix leaves the buffer and its detections become committed; its first matches stay only when LiteLLM would have deferred this cut
fn trimmed(
    source: &ContentFilterSource<'_>,
    state: StreamedChoiceState,
    cut: CutResult,
) -> StreamedChoiceState {
    let mut seen_exception_phrases = state.seen_exception_phrases;
    for phrase in cut.dropped_phrases {
        if !seen_exception_phrases.contains(&phrase) {
            seen_exception_phrases.push(phrase);
        }
    }
    let (open_sentence, carried) = if cut.deferring {
        (
            cut.open_sentence,
            carried_matches(source, &state.carried_matches, cut.head_scan.matches),
        )
    } else {
        (String::new(), CarriedMatches::default())
    };
    StreamedChoiceState {
        buffered_text: cut.tail,
        yielded_masked_text_len: state
            .yielded_masked_text_len
            .saturating_sub(cut.head_scan.masked_len),
        committed_detections: state
            .committed_detections
            .iter()
            .chain(cut.head_scan.detections.iter())
            .cloned()
            .collect(),
        latest_detections: state.latest_detections,
        next_trim_len: 0,
        seen_exception_phrases,
        open_sentence,
        carried_matches: carried,
    }
}

fn carried_matches(
    source: &ContentFilterSource<'_>,
    carried: &CarriedMatches,
    found: CarriedMatches,
) -> CarriedMatches {
    CarriedMatches {
        conditional: earlier_match(
            carried.conditional.as_ref(),
            found.conditional,
            DetectionCheck::Conditional,
            source.categories,
        ),
        phrase: earlier_match(
            carried.phrase.as_ref(),
            found.phrase,
            DetectionCheck::Phrase,
            source.categories,
        ),
        keyword: earlier_match(
            carried.keyword.as_ref(),
            found.keyword,
            DetectionCheck::CategoryKeyword,
            source.categories,
        ),
    }
}

fn committable_cut(
    source: &ContentFilterSource<'_>,
    state: &StreamedChoiceState,
    halves: (&str, &str),
    masked_text: &str,
    suppression: bool,
) -> Option<HeadScan> {
    let (head, tail) = halves;
    let suppressed = CategoryLoadResult::default();
    let scan_source = source_with_categories(source, &suppressed, suppression);
    let mut detections = Vec::new();
    let carried = CarriedMatches::default();
    let input = ScanInput {
        checks: true,
        conditional_text: head,
        text: head,
        carried: &carried,
    };
    let (masked_head, matches) =
        filter_streamed_text(&scan_source, &input, &mut detections).ok()?;
    let masked_tail = filter_single_text(&scan_source, tail, None).ok()?;
    let masked_len = masked_head.chars().count();
    let mut combined = masked_head;
    combined.push_str(&masked_tail);
    if combined != masked_text || masked_len > state.yielded_masked_text_len {
        return None;
    }
    Some(HeadScan {
        detections,
        masked_len,
        matches,
    })
}
