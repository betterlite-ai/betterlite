use std::collections::BTreeMap;
use std::time::Instant;

use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_filter_records::ContentFilterDetection;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::StreamScan;
use crate::content_filter_actions::error::ContentFilterActionError;
use crate::content_filter_actions::{ContentFilterSource, filter_single_text_with_open_sentence};
use crate::content_filter_streaming::STREAM_HOLDBACK_CHARS;
use crate::content_filter_streaming::error::StreamFilterError;
use crate::content_filter_streaming::state::StreamedChoiceState;
use crate::content_filter_streaming::text::char_slice;
use crate::content_filter_streaming::trim::trim_streamed_choice_buffer;

pub(super) struct PendingChoice {
    pub(super) masked_text: String,
    pub(super) state: StreamedChoiceState,
}

struct ChoiceBuffer {
    previous: StreamedChoiceState,
    buffered_text: String,
    text_to_scan: String,
}

pub(super) fn process_chunk(
    item: &mut GuardrailValue,
    scan: &mut StreamScan<'_, '_>,
) -> Result<(), StreamFilterError> {
    let Some(GuardrailValue::Array(choices)) = object_field_mut(item, "choices") else {
        return Ok(());
    };
    for choice in choices.iter_mut() {
        process_choice(choice, scan)?;
    }
    Ok(())
}

fn process_choice(
    choice: &mut GuardrailValue,
    scan: &mut StreamScan<'_, '_>,
) -> Result<(), StreamFilterError> {
    let index = choice_index(choice);
    let is_final = choice_is_final(choice);
    let Some(GuardrailValue::Object(delta)) = object_field_mut(choice, "delta") else {
        return Ok(());
    };
    let Some(pending) = scan_choice(delta, scan, index, is_final)? else {
        return Ok(());
    };
    store_choice(scan, index, is_final, pending);
    Ok(())
}

fn choice_buffer(
    scan: &StreamScan<'_, '_>,
    index: i64,
    content: &str,
    is_final: bool,
) -> Option<ChoiceBuffer> {
    let previous = scan.states.get(&index).cloned().unwrap_or_default();
    let mut buffered_text = previous.buffered_text.clone();
    buffered_text.push_str(content);
    if buffered_text.is_empty() {
        return None;
    }
    let text_to_scan = if is_final {
        format!("{buffered_text} ")
    } else {
        buffered_text.clone()
    };
    Some(ChoiceBuffer {
        previous,
        buffered_text,
        text_to_scan,
    })
}

fn scan_choice(
    delta: &mut BTreeMap<String, GuardrailValue>,
    scan: &mut StreamScan<'_, '_>,
    index: i64,
    is_final: bool,
) -> Result<Option<PendingChoice>, StreamFilterError> {
    let content = match delta.get("content") {
        Some(GuardrailValue::Text(text)) => text.as_str(),
        _ => "",
    };
    if content.is_empty() && !is_final {
        return Ok(None);
    }
    let Some(buffer) = choice_buffer(scan, index, content, is_final) else {
        return Ok(None);
    };
    // LiteLLM strips the synthetic space inside the success path only, so a failed scan keeps it
    let (masked_text, latest_detections) = scan_text(
        scan,
        index,
        &buffer.previous,
        &buffer.text_to_scan,
        is_final,
    )?;
    let yielded_len = write_yielded_content(delta, &masked_text, &buffer.previous, is_final);
    // a sentence end inside the buffer closes the one the previous trim left open
    let open_sentence_words = if buffer.buffered_text.contains(['.', '!', '?']) {
        Vec::new()
    } else {
        buffer.previous.open_sentence_words
    };
    Ok(Some(PendingChoice {
        masked_text,
        state: StreamedChoiceState {
            buffered_text: buffer.buffered_text,
            yielded_masked_text_len: yielded_len,
            committed_detections: buffer.previous.committed_detections,
            latest_detections,
            next_trim_len: buffer.previous.next_trim_len,
            seen_exception_phrases: buffer.previous.seen_exception_phrases,
            open_sentence_words,
        },
    }))
}

// The holdback keeps the last fifty characters back until the final chunk, and the yield never shrinks
fn safe_to_yield_len(masked_text: &str, previous: &StreamedChoiceState, is_final: bool) -> usize {
    let holdback = if is_final { 0 } else { STREAM_HOLDBACK_CHARS };
    masked_text
        .chars()
        .count()
        .saturating_sub(holdback)
        .max(previous.yielded_masked_text_len)
}

fn write_yielded_content(
    delta: &mut BTreeMap<String, GuardrailValue>,
    masked_text: &str,
    previous: &StreamedChoiceState,
    is_final: bool,
) -> usize {
    let yielded_len = safe_to_yield_len(masked_text, previous, is_final);
    let yielded = char_slice(masked_text, previous.yielded_masked_text_len, yielded_len);
    delta.insert(
        "content".to_owned(),
        GuardrailValue::Text(yielded.to_owned()),
    );
    yielded_len
}

fn scan_text(
    scan: &mut StreamScan<'_, '_>,
    index: i64,
    previous: &StreamedChoiceState,
    text_to_scan: &str,
    is_final: bool,
) -> Result<(String, Vec<ContentFilterDetection>), StreamFilterError> {
    let mut choice_detections = Vec::new();
    let started = Instant::now();
    // Python's buffer still holds the phrases and the open sentence a trim dropped, so its scan
    // suppresses every category match and joins them to this text
    let empty_categories = CategoryLoadResult::default();
    let categories = if previous.seen_exception_phrases.is_empty() {
        scan.source.categories
    } else {
        &empty_categories
    };
    let filter_source = ContentFilterSource {
        categories,
        patterns: scan.source.patterns,
        blocked_words: scan.source.blocked_words,
        pattern_redaction_format: scan.source.pattern_redaction_format,
        keyword_redaction_tag: scan.source.keyword_redaction_tag,
    };
    let open_sentence = previous.open_sentence_words.join(" ");
    let carried = if open_sentence.is_empty() {
        None
    } else {
        Some(open_sentence.as_str())
    };
    let outcome = filter_single_text_with_open_sentence(
        &filter_source,
        text_to_scan,
        carried,
        Some(&mut choice_detections),
    );
    scan.scan_time = scan.scan_time.saturating_add(started.elapsed());
    match outcome {
        Ok(mut masked) => {
            if is_final && masked.ends_with(' ') {
                masked.pop();
            }
            Ok((masked, choice_detections))
        }
        Err(ContentFilterActionError::Blocked { detail }) => {
            scan.states.insert(
                index,
                StreamedChoiceState {
                    buffered_text: previous.buffered_text.clone(),
                    yielded_masked_text_len: previous.yielded_masked_text_len,
                    committed_detections: previous.committed_detections.clone(),
                    latest_detections: choice_detections,
                    next_trim_len: previous.next_trim_len,
                    seen_exception_phrases: previous.seen_exception_phrases.clone(),
                    open_sentence_words: previous.open_sentence_words.clone(),
                },
            );
            Err(StreamFilterError::Blocked { detail })
        }
        Err(error) => {
            tracing::error!(
                %error,
                "content filter streaming scan failed; yielding the unfiltered text"
            );
            Ok((text_to_scan.to_owned(), previous.latest_detections.clone()))
        }
    }
}

pub(super) fn store_choice(
    scan: &mut StreamScan<'_, '_>,
    index: i64,
    is_final: bool,
    pending: PendingChoice,
) {
    if is_final {
        scan.states.insert(index, pending.state);
        return;
    }
    let started = Instant::now();
    let trimmed =
        trim_streamed_choice_buffer(scan.source, pending.state, &pending.masked_text, &scan.plan);
    scan.scan_time = scan.scan_time.saturating_add(started.elapsed());
    scan.states.insert(index, trimmed);
}

fn choice_field<'choice>(
    choice: &'choice GuardrailValue,
    key: &str,
) -> Option<&'choice GuardrailValue> {
    let GuardrailValue::Object(fields) = choice else {
        return None;
    };
    fields.get(key)
}

fn choice_index(choice: &GuardrailValue) -> i64 {
    match choice_field(choice, "index") {
        Some(GuardrailValue::SignedInteger(index)) => *index,
        Some(GuardrailValue::UnsignedInteger(index)) => i64::try_from(*index).unwrap_or(i64::MAX),
        _ => 0,
    }
}

fn choice_is_final(choice: &GuardrailValue) -> bool {
    matches!(
        choice_field(choice, "finish_reason"),
        Some(GuardrailValue::Text(reason)) if !reason.is_empty()
    )
}

fn object_field_mut<'value>(
    value: &'value mut GuardrailValue,
    key: &str,
) -> Option<&'value mut GuardrailValue> {
    let GuardrailValue::Object(fields) = value else {
        return None;
    };
    fields.get_mut(key)
}
