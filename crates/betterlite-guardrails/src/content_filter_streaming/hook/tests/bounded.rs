use std::time::Duration;

use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_category::model::CategoryConfig;
use betterlite_config::sections::content_filter_records::category_types::ConditionalCategoryConfig;
use betterlite_config::sections::content_filter_records::{
    ContentFilterAction, ContentFilterDetection,
};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;

use super::{
    StreamCase, choice, choice_of, chunk, content_at, masked_word, record, run_stream, stream_case,
    stream_source, text_field,
};
use crate::content_filter_actions::error::ContentFilterActionError;
use crate::content_filter_actions::filter_single_text;
use crate::content_filter_streaming::STREAM_HOLDBACK_CHARS;
use crate::content_filter_streaming::error::StreamFilterError;
use crate::content_filter_streaming::hook::StreamScan;
use crate::content_filter_streaming::hook::scan::process_chunk;
use crate::content_filter_streaming::merge::merge_detections;
use crate::content_filter_streaming::plan::streamed_scan_plan;
use crate::content_filter_streaming::state::StreamedChoiceState;
use crate::content_filter_streaming::text::char_slice;

struct DrivenStream {
    chunks: Vec<GuardrailValue>,
    states: IndexMap<i64, StreamedChoiceState>,
    max_buffered_len: usize,
    context_chars: usize,
}

// The hook's chunk loop without the channels, so a test can read every state between chunks
fn drive_chunks(case: &StreamCase) -> DrivenStream {
    let source = stream_source(case);
    let plan = streamed_scan_plan(&source);
    let context_chars = plan.context_chars;
    let mut scan = StreamScan {
        source: &source,
        plan,
        states: IndexMap::new(),
        scan_time: Duration::ZERO,
    };
    let mut chunks = Vec::with_capacity(case.items.len());
    let mut max_buffered_len = 0;
    for item in &case.items {
        let mut item = item.clone();
        process_chunk(&mut item, &mut scan).expect("the stream does not block");
        for state in scan.states.values() {
            max_buffered_len = max_buffered_len.max(state.buffered_text.chars().count());
        }
        chunks.push(item);
    }
    DrivenStream {
        chunks,
        states: scan.states,
        max_buffered_len,
        context_chars,
    }
}

// What Python's never-trimmed buffer yields and detects, the reference the trimmed stream must equal
fn untrimmed(case: &StreamCase) -> (Vec<String>, Vec<ContentFilterDetection>) {
    let source = stream_source(case);
    let mut buffer = String::new();
    let mut yielded = 0;
    let mut latest = Vec::new();
    let mut contents = Vec::with_capacity(case.items.len());
    for item in &case.items {
        let choice = choice_of(item, 0);
        let is_final =
            text_field(choice, &["finish_reason"]).is_some_and(|reason| !reason.is_empty());
        let content = text_field(choice, &["delta", "content"]).unwrap_or_default();
        buffer.push_str(content);
        if buffer.is_empty() {
            contents.push(String::new());
            continue;
        }
        let text_to_scan = if is_final {
            format!("{buffer} ")
        } else {
            buffer.clone()
        };
        let mut detections = Vec::new();
        let mut masked = filter_single_text(&source, &text_to_scan, Some(&mut detections))
            .expect("the untrimmed stream does not block");
        if is_final && masked.ends_with(' ') {
            masked.pop();
        }
        let holdback = if is_final { 0 } else { STREAM_HOLDBACK_CHARS };
        let safe = masked.chars().count().saturating_sub(holdback).max(yielded);
        contents.push(char_slice(&masked, yielded, safe).to_owned());
        yielded = safe;
        latest = detections;
    }
    (contents, merge_detections(&latest))
}

fn chunk_contents(chunks: &[GuardrailValue]) -> Vec<String> {
    (0..chunks.len())
        .map(|position| content_at(&chunks[position], 0))
        .collect()
}

fn conditional_categories() -> CategoryLoadResult {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "stream_policy".to_owned(),
        ConditionalCategoryConfig {
            identifier_words: vec!["attack".to_owned()],
            block_words: vec!["target".to_owned()],
            action: ContentFilterAction::Block,
            severity: "high".to_owned(),
        },
    );
    categories
}

#[test]
fn hook_keeps_the_buffer_bounded_while_an_exception_phrase_stays_behind_the_tail() {
    let mut categories = CategoryLoadResult::default();
    categories.loaded_categories.insert(
        "stream_policy".to_owned(),
        CategoryConfig {
            category_name: "stream_policy".to_owned(),
            description: String::new(),
            default_action: ContentFilterAction::Mask,
            keywords: Vec::new(),
            exceptions: vec!["allowed phrase".to_owned()],
            identifier_words: Vec::new(),
            always_block_keywords: Vec::new(),
            inherit_from: None,
            additional_block_words: Vec::new(),
            phrase_patterns: Vec::new(),
        },
    );
    categories.category_keywords.insert(
        "target".to_owned(),
        (
            "stream_policy".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Mask,
        ),
    );
    let mut items = Vec::new();
    let opening = format!("allowed phrase {}", "a".repeat(85));
    items.push(chunk(vec![choice(Some(0), Some(opening.as_str()), None)]));
    for index in 0..40 {
        let content = if index == 2 {
            format!("secret{}", "x".repeat(94))
        } else {
            "x".repeat(100)
        };
        items.push(chunk(vec![choice(Some(0), Some(content.as_str()), None)]));
    }
    items.push(chunk(vec![choice(
        Some(0),
        Some(" target the end"),
        Some("stop"),
    )]));
    let mut case = stream_case(items);
    case.categories = categories;
    case.blocked_words = vec![masked_word("secret")];
    let driven = drive_chunks(&case);
    assert!(driven.max_buffered_len <= driven.context_chars * 2);
    let state = driven.states.get(&0).expect("choice 0 has a state");
    assert_eq!(
        state.seen_exception_phrases,
        vec!["allowed phrase".to_owned()]
    );
    let detections: Vec<_> = state
        .committed_detections
        .iter()
        .chain(&state.latest_detections)
        .cloned()
        .collect();
    let ours = merge_detections(&detections);
    let (expected_contents, expected_detections) = untrimmed(&case);
    assert_eq!(chunk_contents(&driven.chunks), expected_contents);
    assert_eq!(
        serde_json::to_value(&ours).unwrap(),
        serde_json::to_value(&expected_detections).unwrap()
    );
    // the phrase left the buffer long ago, so the keyword after it stays unmasked as before
    assert!(
        expected_contents
            .iter()
            .any(|content| content.contains("target"))
    );
}

#[test]
fn hook_keeps_the_buffer_bounded_while_an_open_conditional_sentence_has_no_end() {
    let mut items = Vec::new();
    let opening = format!("attack {}", "a".repeat(93));
    items.push(chunk(vec![choice(Some(0), Some(opening.as_str()), None)]));
    for index in 0..40 {
        let content = if index == 2 {
            format!("secret{}", "x".repeat(94))
        } else {
            "x".repeat(100)
        };
        items.push(chunk(vec![choice(Some(0), Some(content.as_str()), None)]));
    }
    items.push(chunk(vec![choice(Some(0), Some(" the end"), Some("stop"))]));
    let mut case = stream_case(items);
    case.categories = conditional_categories();
    case.blocked_words = vec![masked_word("secret")];
    let driven = drive_chunks(&case);
    assert!(driven.max_buffered_len <= driven.context_chars * 2);
    let state = driven.states.get(&0).expect("choice 0 has a state");
    assert_eq!(state.open_sentence_words, vec!["attack".to_owned()]);
    let detections: Vec<_> = state
        .committed_detections
        .iter()
        .chain(&state.latest_detections)
        .cloned()
        .collect();
    let ours = merge_detections(&detections);
    let (expected_contents, expected_detections) = untrimmed(&case);
    assert_eq!(chunk_contents(&driven.chunks), expected_contents);
    assert_eq!(
        serde_json::to_value(&ours).unwrap(),
        serde_json::to_value(&expected_detections).unwrap()
    );
}

#[tokio::test]
async fn hook_keeps_an_open_conditional_sentence_when_the_trim_drops_it() {
    let opening = format!("attack {}", "x".repeat(1093));
    let mut case = stream_case(vec![
        chunk(vec![choice(Some(0), Some(opening.as_str()), None)]),
        chunk(vec![choice(Some(0), Some(" target"), Some("stop"))]),
    ]);
    case.categories = conditional_categories();
    let run = run_stream(&case).await;
    let Err(StreamFilterError::Blocked { detail }) = &run.outcome else {
        panic!("the trim dropped the conditional identifier");
    };
    let whole = format!("{opening} target ");
    let Err(ContentFilterActionError::Blocked { detail: expected }) =
        filter_single_text(&stream_source(&case), &whole, None)
    else {
        panic!("the untrimmed scan blocks");
    };
    assert_eq!(detail, &expected);
    assert_eq!(
        text_field(record(&run), &["guardrail_status"]),
        Some("guardrail_intervened")
    );
}
