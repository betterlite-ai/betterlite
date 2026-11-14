use betterlite_config::sections::content_filter_records::ContentFilterDetection;

use crate::content_filter_actions::{ContentFilterSource, filter_single_text};
use crate::content_filter_streaming::context::cut_breaks_wider_context;
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
    let cut = committable_cut(source, &state, (head, tail), masked_text, plan);
    let tail_text = tail.to_owned();
    let next_trim_len = buffered_len.saturating_add(plan.context_chars);
    match cut {
        None => deferred(state, next_trim_len),
        Some((head_detections, masked_head_len)) => {
            trimmed(state, tail_text, &head_detections, masked_head_len)
        }
    }
}

// The buffer stays whole and only the bar for the next attempt moves, one context past its length
fn deferred(state: StreamedChoiceState, next_trim_len: usize) -> StreamedChoiceState {
    StreamedChoiceState {
        buffered_text: state.buffered_text,
        yielded_masked_text_len: state.yielded_masked_text_len,
        committed_detections: state.committed_detections,
        latest_detections: state.latest_detections,
        next_trim_len,
    }
}

// The dropped prefix leaves the buffer, its detections become committed, and the yielded count drops by it
fn trimmed(
    state: StreamedChoiceState,
    tail: String,
    head_detections: &[ContentFilterDetection],
    masked_head_len: usize,
) -> StreamedChoiceState {
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
    }
}

fn committable_cut(
    source: &ContentFilterSource<'_>,
    state: &StreamedChoiceState,
    halves: (&str, &str),
    masked_text: &str,
    plan: &StreamedScanPlan,
) -> Option<(Vec<ContentFilterDetection>, usize)> {
    let (head, tail) = halves;
    if cut_breaks_wider_context(&state.buffered_text, head, tail, plan) {
        return None;
    }
    let mut head_detections = Vec::new();
    let masked_head = filter_single_text(source, head, Some(&mut head_detections)).ok()?;
    let masked_tail = filter_single_text(source, tail, None).ok()?;
    let masked_head_len = masked_head.chars().count();
    let mut combined = masked_head;
    combined.push_str(&masked_tail);
    if combined != masked_text || masked_head_len > state.yielded_masked_text_len {
        return None;
    }
    Some((head_detections, masked_head_len))
}
