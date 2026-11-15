use std::iter::repeat_n;
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
    StreamCase, choice, choice_of, chunk, compiled_pattern, content_at, masked_word, record,
    response_detections, run_stream, stream_case, stream_source, text_field,
};
use crate::content_filter_actions::error::{ContentFilterActionError, ContentFilterBlockDetail};
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
    max_carried_len: usize,
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
    let mut max_carried_len = 0;
    for item in &case.items {
        let mut outgoing = item.clone();
        process_chunk(&mut outgoing, &mut scan).expect("the stream does not block");
        for state in scan.states.values() {
            max_buffered_len = max_buffered_len.max(state.buffered_text.chars().count());
            max_carried_len = max_carried_len.max(state.open_sentence.chars().count());
        }
        chunks.push(outgoing);
    }
    DrivenStream {
        chunks,
        states: scan.states,
        max_buffered_len,
        max_carried_len,
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
        let payload = choice_of(item, 0);
        let is_final =
            text_field(payload, &["finish_reason"]).is_some_and(|reason| !reason.is_empty());
        let content = text_field(payload, &["delta", "content"]).unwrap_or_default();
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

// The record's order: category checks first, then patterns and blocked words in config order
fn ordered_detections(state: &StreamedChoiceState) -> Vec<ContentFilterDetection> {
    let mut ordered: Vec<_> = state
        .committed_detections
        .iter()
        .chain(&state.latest_detections)
        .cloned()
        .collect();
    ordered.sort_by_key(|entry| (entry.check, entry.position));
    ordered.into_iter().map(|entry| entry.detection).collect()
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

fn child_categories(action: ContentFilterAction, severity: &str) -> CategoryLoadResult {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "stream_policy".to_owned(),
        ConditionalCategoryConfig {
            identifier_words: vec!["child".to_owned()],
            block_words: vec!["harm".to_owned()],
            action,
            severity: severity.to_owned(),
        },
    );
    categories
}

fn exception_category() -> CategoryLoadResult {
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
    categories
}

fn insert_keywords(categories: &mut CategoryLoadResult) {
    for (keyword, category, action) in [
        ("alpha", "alpha_policy", ContentFilterAction::Mask),
        ("beta", "beta_policy", ContentFilterAction::Block),
    ] {
        categories.category_keywords.insert(
            keyword.to_owned(),
            (category.to_owned(), "high".to_owned(), action),
        );
    }
}

// One opening chunk, the padding chunks, then a final chunk that ends the stream
fn padding_items(opening: &str, padding: &[String], final_text: &str) -> Vec<GuardrailValue> {
    let mut items = vec![chunk(vec![choice(Some(0), Some(opening), None)])];
    items.extend(
        padding
            .iter()
            .map(|content| chunk(vec![choice(Some(0), Some(content.as_str()), None)])),
    );
    items.push(chunk(vec![choice(Some(0), Some(final_text), Some("stop"))]));
    items
}

fn assert_buffer_bounded(driven: &DrivenStream) {
    let bar = driven.context_chars.saturating_mul(2);
    assert!(driven.max_buffered_len <= bar);
    assert!(driven.max_carried_len <= bar);
}

fn assert_stream_matches_untrimmed(case: &StreamCase, driven: &DrivenStream) {
    let state = driven.states.get(&0).expect("choice 0 has a state");
    let ours = merge_detections(&ordered_detections(state));
    let (expected_contents, expected_detections) = untrimmed(case);
    assert_eq!(chunk_contents(&driven.chunks), expected_contents);
    assert_eq!(
        serde_json::to_value(&ours).expect("the detections serialize"),
        serde_json::to_value(&expected_detections).expect("the detections serialize")
    );
}

#[test]
fn hook_keeps_the_buffer_bounded_while_an_exception_phrase_stays_behind_the_tail() {
    let mut categories = exception_category();
    categories.category_keywords.insert(
        "target".to_owned(),
        (
            "stream_policy".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Mask,
        ),
    );
    let opening = format!("allowed phrase {}", "a".repeat(85));
    let mut padding = vec!["x".repeat(100); 40];
    padding[2] = format!("secret{}", "x".repeat(94));
    let mut case = stream_case(padding_items(&opening, &padding, " target the end"));
    case.categories = categories;
    case.blocked_words = vec![masked_word("secret")];
    let driven = drive_chunks(&case);
    assert_buffer_bounded(&driven);
    let state = driven.states.get(&0).expect("choice 0 has a state");
    assert_eq!(
        state.seen_exception_phrases,
        vec!["allowed phrase".to_owned()]
    );
    assert_stream_matches_untrimmed(&case, &driven);
    // the phrase left the buffer long ago, so the keyword after it stays unmasked as before
    assert!(
        chunk_contents(&driven.chunks)
            .iter()
            .any(|content| content.contains("target"))
    );
}

#[tokio::test]
async fn hook_does_not_mask_a_keyword_while_the_phrase_is_in_the_buffer() {
    let mut categories = exception_category();
    categories.category_keywords.insert(
        "alpha".to_owned(),
        (
            "other_policy".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Mask,
        ),
    );
    let opening = format!("allowed phrase alpha {}", "a".repeat(900));
    let padding = vec!["x".repeat(100); 2];
    let mut case = stream_case(padding_items(&opening, &padding, " done"));
    case.categories = categories;
    let run = run_stream(&case).await;
    assert!(run.outcome.is_ok(), "the stream has no block");
    let (expected_contents, expected_detections) = untrimmed(&case);
    assert_eq!(chunk_contents(&run.chunks), expected_contents);
    assert!(expected_detections.is_empty());
    assert!(response_detections(record(&run)).is_empty());
}

#[test]
fn hook_keeps_the_buffer_bounded_while_an_open_conditional_sentence_has_no_end() {
    let sentence = format!("attack {}", "a".repeat(93));
    let mut items: Vec<GuardrailValue> = repeat_n(sentence.as_str(), 41)
        .map(|content| chunk(vec![choice(Some(0), Some(content), None)]))
        .collect();
    items.push(chunk(vec![choice(Some(0), Some(" the end"), Some("stop"))]));
    let mut case = stream_case(items);
    case.categories = conditional_categories();
    let driven = drive_chunks(&case);
    assert_buffer_bounded(&driven);
    let state = driven.states.get(&0).expect("choice 0 has a state");
    assert!(!state.open_sentence.is_empty());
    assert_stream_matches_untrimmed(&case, &driven);
}

#[test]
fn hook_keeps_a_block_word_inside_a_longer_word_of_the_carried_sentence() {
    let opening = format!("child harmless {}", "a".repeat(900));
    let padding = vec!["x".repeat(100); 2];
    let mut case = stream_case(padding_items(&opening, &padding, " more text"));
    case.categories = child_categories(ContentFilterAction::Block, "high");
    let driven = drive_chunks(&case);
    assert_buffer_bounded(&driven);
    let state = driven.states.get(&0).expect("choice 0 has a state");
    assert!(state.open_sentence.contains("harmless"));
    assert_stream_matches_untrimmed(&case, &driven);
}

#[tokio::test]
async fn hook_does_not_pair_a_block_word_the_carried_truncation_splits() {
    let opening = format!("aharm {}child", "w".repeat(1526));
    let mut case = stream_case(vec![
        chunk(vec![choice(Some(0), Some(opening.as_str()), None)]),
        chunk(vec![choice(Some(0), Some(" done"), Some("stop"))]),
    ]);
    case.categories = child_categories(ContentFilterAction::Block, "high");
    let run = run_stream(&case).await;
    assert!(run.outcome.is_ok(), "the split word never pairs with child");
    let (expected_contents, expected_detections) = untrimmed(&case);
    assert_eq!(chunk_contents(&run.chunks), expected_contents);
    assert!(expected_detections.is_empty());
    assert!(response_detections(record(&run)).is_empty());
}

#[test]
fn hook_keeps_the_dropped_prefixs_first_keyword_ahead_of_a_later_block() {
    let opening = format!("alpha child {}", "x".repeat(955));
    let mut categories = child_categories(ContentFilterAction::Mask, "medium");
    insert_keywords(&mut categories);
    let padding = vec!["x".repeat(100); 2];
    let mut case = stream_case(padding_items(&opening, &padding, "beta done"));
    case.categories = categories;
    let driven = drive_chunks(&case);
    let state = driven.states.get(&0).expect("choice 0 has a state");
    let carried = state
        .carried_matches
        .keyword
        .as_ref()
        .expect("the dropped keyword is carried");
    assert_eq!(carried.matched, "alpha");
    assert_stream_matches_untrimmed(&case, &driven);
}

#[tokio::test]
async fn hook_drops_a_trimmed_keyword_when_litellm_would_commit_the_cut() {
    let opening = format!("alpha {}", "x".repeat(960));
    let mut categories = CategoryLoadResult::default();
    insert_keywords(&mut categories);
    let padding = vec!["x".repeat(100); 2];
    let mut case = stream_case(padding_items(&opening, &padding, " beta done"));
    case.categories = categories;
    let run = run_stream(&case).await;
    let Err(StreamFilterError::Blocked { detail }) = &run.outcome else {
        panic!("the later beta keyword is the first match once alpha leaves the buffer");
    };
    let ContentFilterBlockDetail::CategoryKeyword { keyword, .. } = detail else {
        panic!("the block comes from a category keyword");
    };
    assert_eq!(keyword.as_str(), "beta");
    assert_eq!(
        text_field(record(&run), &["guardrail_status"]),
        Some("guardrail_intervened")
    );
}

#[tokio::test]
async fn hook_stops_masking_a_carried_keyword_once_the_phrase_is_dropped() {
    let mut categories = exception_category();
    insert_keywords(&mut categories);
    let opening = format!("alpha {}", "a".repeat(900));
    let padding = "b".repeat(200);
    let phrase = format!(" allowed phrase {}", "c".repeat(700));
    let mut case = stream_case(vec![
        chunk(vec![choice(Some(0), Some(opening.as_str()), None)]),
        chunk(vec![choice(Some(0), Some(padding.as_str()), None)]),
        chunk(vec![choice(Some(0), Some(phrase.as_str()), None)]),
        chunk(vec![choice(Some(0), Some(" alpha again "), Some("stop"))]),
    ]);
    case.categories = categories;
    let run = run_stream(&case).await;
    // LiteLLM commits the cut that drops the masked alpha, so its held-back text moves with the mask
    let contents = chunk_contents(&run.chunks);
    let lengths: Vec<usize> = contents
        .iter()
        .map(|content| content.chars().count())
        .collect();
    assert_eq!(lengths, vec![869, 200, 716, 63]);
    assert_eq!(
        contents[2],
        format!(
            "{}{}{}",
            "b".repeat(50),
            " allowed phrase ",
            "c".repeat(650)
        )
    );
    // Python's buffer keeps the phrase, so its keyword check stays skipped and masks no later alpha
    assert!(content_at(&run.chunks[3], 0).contains(" alpha again "));
    let detections = response_detections(record(&run));
    assert_eq!(detections.len(), 1);
    assert_eq!(
        text_field(&detections[0], &["type"]),
        Some("category_keyword")
    );
    assert_eq!(text_field(&detections[0], &["keyword"]), Some("alpha"));
}

#[tokio::test]
async fn hook_records_detections_in_whole_buffer_scan_order_after_a_trim() {
    let opening = format!("earlypat earlyword {}", "a".repeat(1080));
    let mut items = vec![chunk(vec![choice(Some(0), Some(opening.as_str()), None)])];
    items.push(chunk(vec![choice(
        Some(0),
        Some(" latepat lateword"),
        Some("stop"),
    )]));
    let mut case = stream_case(items);
    case.patterns = vec![
        compiled_pattern("latepat", "late"),
        compiled_pattern("earlypat", "early"),
    ];
    case.blocked_words = vec![masked_word("lateword"), masked_word("earlyword")];
    let run = run_stream(&case).await;
    let (expected_contents, _) = untrimmed(&case);
    assert_eq!(chunk_contents(&run.chunks), expected_contents);
    // a whole-buffer scan finds the patterns and blocked words in config order, whichever half holds them
    let detections = response_detections(record(&run));
    assert_eq!(detections.len(), 4);
    assert_eq!(text_field(&detections[0], &["type"]), Some("pattern"));
    assert_eq!(text_field(&detections[0], &["pattern_name"]), Some("late"));
    assert_eq!(text_field(&detections[1], &["pattern_name"]), Some("early"));
    assert_eq!(text_field(&detections[2], &["type"]), Some("blocked_word"));
    assert_eq!(text_field(&detections[2], &["keyword"]), Some("lateword"));
    assert_eq!(text_field(&detections[3], &["keyword"]), Some("earlyword"));
}

#[tokio::test]
async fn hook_records_a_later_category_keyword_before_a_committed_pattern() {
    let opening = format!("earlypat {}", "a".repeat(1080));
    let padding = "b".repeat(200);
    let mut case = stream_case(vec![
        chunk(vec![choice(Some(0), Some(opening.as_str()), None)]),
        chunk(vec![choice(Some(0), Some(padding.as_str()), None)]),
        chunk(vec![choice(Some(0), Some(" keywordword"), Some("stop"))]),
    ]);
    case.patterns = vec![compiled_pattern("earlypat", "early")];
    case.categories.category_keywords.insert(
        "keywordword".to_owned(),
        (
            "kw_policy".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Mask,
        ),
    );
    let run = run_stream(&case).await;
    let (expected_contents, _) = untrimmed(&case);
    assert_eq!(chunk_contents(&run.chunks), expected_contents);
    // the category check runs before the patterns, though its detection came from the later chunk
    let detections = response_detections(record(&run));
    assert_eq!(detections.len(), 2);
    assert_eq!(
        text_field(&detections[0], &["type"]),
        Some("category_keyword")
    );
    assert_eq!(
        text_field(&detections[0], &["keyword"]),
        Some("keywordword")
    );
    assert_eq!(text_field(&detections[1], &["type"]), Some("pattern"));
    assert_eq!(text_field(&detections[1], &["pattern_name"]), Some("early"));
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
