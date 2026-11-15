#![cfg(test)]

mod bounded;

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_filter_records::{
    BlockedWord, ContentFilterAction, ContentFilterPattern,
};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use indexmap::IndexMap;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::scan::{PendingChoice, store_choice};
use super::{
    StreamFilterCall, StreamFilterSettings, StreamScan, stream_filter_hook, stream_status,
};
use crate::content_filter::risk_scoring::GuardrailStatus;
use crate::content_filter_actions::ContentFilterSource;
use crate::content_filter_actions::error::ContentFilterBlockDetail;
use crate::content_filter_streaming::error::StreamFilterError;
use crate::content_filter_streaming::logging::log_stream_record;
use crate::content_filter_streaming::plan::streamed_scan_plan;
use crate::content_filter_streaming::state::{CarriedMatches, StreamedChoiceState};
use crate::guardrail_records::error::GuardrailRecordError;
use crate::pattern_matching::{CompiledPattern, compile};

struct StreamCase {
    settings: StreamFilterSettings,
    categories: CategoryLoadResult,
    patterns: Vec<CompiledPattern>,
    blocked_words: Vec<BlockedWord>,
    items: Vec<GuardrailValue>,
    user_api_key_dict: Option<GuardrailValue>,
    close_after: Option<usize>,
}

struct StreamRun {
    chunks: Vec<GuardrailValue>,
    request_data: BTreeMap<String, GuardrailValue>,
    outcome: Result<(), StreamFilterError>,
}

fn stream_settings() -> StreamFilterSettings {
    StreamFilterSettings {
        guardrail_name: "stream-filter".to_owned(),
        event_hook: LitellmParamsMode::One("post_call".to_owned()),
        guardrail_id: None,
        policy_template: None,
        pattern_redaction_format: "[{pattern_name}_REDACTED]".to_owned(),
        keyword_redaction_tag: "[KEYWORD_REDACTED]".to_owned(),
    }
}

fn stream_case(items: Vec<GuardrailValue>) -> StreamCase {
    StreamCase {
        settings: stream_settings(),
        categories: CategoryLoadResult::default(),
        patterns: Vec::new(),
        blocked_words: Vec::new(),
        items,
        user_api_key_dict: None,
        close_after: None,
    }
}

const fn stream_source(case: &StreamCase) -> ContentFilterSource<'_> {
    ContentFilterSource {
        categories: &case.categories,
        patterns: case.patterns.as_slice(),
        blocked_words: case.blocked_words.as_slice(),
        pattern_redaction_format: case.settings.pattern_redaction_format.as_str(),
        keyword_redaction_tag: case.settings.keyword_redaction_tag.as_str(),
    }
}

// the sender moves into the feeder, so the input channel closes when the last item is sent
async fn feed_items(items: &[GuardrailValue], input: mpsc::Sender<GuardrailValue>) {
    for item in items {
        if input.send(item.clone()).await.is_err() {
            break;
        }
    }
}

// the receiver is owned and dropped here, so a stopped consumer really disconnects the hook
async fn collect_chunks(
    mut relayed: mpsc::Receiver<GuardrailValue>,
    close_after: Option<usize>,
) -> Vec<GuardrailValue> {
    let limit = close_after.unwrap_or(usize::MAX);
    let mut chunks = Vec::new();
    while let Some(item) = relayed.recv().await {
        chunks.push(item);
        if chunks.len() >= limit {
            break;
        }
    }
    chunks
}

async fn run_stream(case: &StreamCase) -> StreamRun {
    let source = stream_source(case);
    let (input, response) = mpsc::channel(case.items.len().max(1));
    let (output, relayed) = mpsc::channel(1);
    let user_api_key_dict = case.user_api_key_dict.as_ref();
    let feed = feed_items(&case.items, input);
    let consume = collect_chunks(relayed, case.close_after);
    let mut request_data = BTreeMap::new();
    let (outcome, (), chunks) = {
        let mut call = StreamFilterCall {
            user_api_key_dict,
            request_data: &mut request_data,
            log_target: None,
        };
        let hook = stream_filter_hook(response, output, &case.settings, &source, &mut call);
        timeout(Duration::from_secs(5), async move {
            tokio::join!(hook, feed, consume)
        })
        .await
        .expect("the stream hook finishes")
    };
    StreamRun {
        chunks,
        request_data,
        outcome,
    }
}

fn chunk(choices: Vec<GuardrailValue>) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        (
            "id".to_owned(),
            GuardrailValue::Text("chatcmpl-stream".to_owned()),
        ),
        ("choices".to_owned(), GuardrailValue::Array(choices)),
    ]))
}

fn choice(
    index: Option<i64>,
    content: Option<&str>,
    finish_reason: Option<&str>,
) -> GuardrailValue {
    choice_with_index(
        index.map(GuardrailValue::SignedInteger),
        content,
        finish_reason,
    )
}

fn choice_with_index(
    index: Option<GuardrailValue>,
    content: Option<&str>,
    finish_reason: Option<&str>,
) -> GuardrailValue {
    let mut fields = BTreeMap::new();
    if let Some(index) = index {
        fields.insert("index".to_owned(), index);
    }
    let mut delta = BTreeMap::new();
    if let Some(content) = content {
        delta.insert(
            "content".to_owned(),
            GuardrailValue::Text(content.to_owned()),
        );
    }
    fields.insert("delta".to_owned(), GuardrailValue::Object(delta));
    fields.insert(
        "finish_reason".to_owned(),
        match finish_reason {
            Some(reason) => GuardrailValue::Text(reason.to_owned()),
            None => GuardrailValue::Null,
        },
    );
    GuardrailValue::Object(fields)
}

fn word(keyword: &str, action: ContentFilterAction) -> BlockedWord {
    BlockedWord {
        keyword: keyword.to_owned(),
        action,
        description: None,
    }
}

fn masked_word(keyword: &str) -> BlockedWord {
    word(keyword, ContentFilterAction::Mask)
}

fn compiled_pattern(pattern: &str, name: &str) -> CompiledPattern {
    compile(&ContentFilterPattern {
        pattern_type: "regex".to_owned(),
        pattern_name: None,
        pattern: Some(pattern.to_owned()),
        name: Some(name.to_owned()),
        action: ContentFilterAction::Mask,
    })
    .expect("the test pattern compiles")
}

fn pending(buffer: &str, yielded: usize) -> PendingChoice {
    PendingChoice {
        masked_text: buffer.to_owned(),
        state: StreamedChoiceState {
            buffered_text: buffer.to_owned(),
            yielded_masked_text_len: yielded,
            committed_detections: Vec::new(),
            latest_detections: Vec::new(),
            next_trim_len: 0,
            seen_exception_phrases: Vec::new(),
            open_sentence: String::new(),
            carried_matches: CarriedMatches::default(),
        },
    }
}

fn field<'value>(value: &'value GuardrailValue, path: &[&str]) -> Option<&'value GuardrailValue> {
    let mut current = value;
    for key in path {
        let GuardrailValue::Object(fields) = current else {
            return None;
        };
        current = fields.get(*key)?;
    }
    Some(current)
}

fn text_field<'value>(value: &'value GuardrailValue, path: &[&str]) -> Option<&'value str> {
    match field(value, path) {
        Some(GuardrailValue::Text(text)) => Some(text),
        _ => None,
    }
}

fn int_field(value: &GuardrailValue, path: &[&str]) -> Option<i64> {
    match field(value, path) {
        Some(GuardrailValue::SignedInteger(number)) => Some(*number),
        Some(GuardrailValue::UnsignedInteger(number)) => i64::try_from(*number).ok(),
        _ => None,
    }
}

fn float_field(value: &GuardrailValue, path: &[&str]) -> Option<f64> {
    match field(value, path) {
        Some(GuardrailValue::Decimal(number)) => Some(*number),
        _ => None,
    }
}

fn choice_of(chunk: &GuardrailValue, position: usize) -> &GuardrailValue {
    let Some(GuardrailValue::Array(choices)) = field(chunk, &["choices"]) else {
        panic!("the chunk has no choices array");
    };
    &choices[position]
}

fn content_at(chunk: &GuardrailValue, position: usize) -> String {
    match field(choice_of(chunk, position), &["delta", "content"]) {
        Some(GuardrailValue::Text(text)) => text.clone(),
        _ => String::new(),
    }
}

fn record(run: &StreamRun) -> &GuardrailValue {
    let Some(GuardrailValue::Object(metadata)) = run.request_data.get("metadata") else {
        panic!("the hook wrote no request metadata");
    };
    let Some(GuardrailValue::Array(records)) =
        metadata.get("standard_logging_guardrail_information")
    else {
        panic!("the hook wrote no guardrail record");
    };
    &records[0]
}

fn response_detections(record: &GuardrailValue) -> &[GuardrailValue] {
    let Some(GuardrailValue::Array(detections)) = field(record, &["guardrail_response"]) else {
        panic!("the record has no detection array");
    };
    detections
}

#[tokio::test]
async fn hook_holds_back_the_last_fifty_characters_of_a_non_final_chunk() {
    let first = "a".repeat(60);
    let second = "b".repeat(60);
    let case = stream_case(vec![
        chunk(vec![choice(Some(0), Some(first.as_str()), None)]),
        chunk(vec![choice(Some(0), Some(second.as_str()), None)]),
        chunk(vec![choice(Some(0), None, Some("stop"))]),
    ]);
    let run = run_stream(&case).await;
    assert_eq!(run.chunks.len(), 3);
    assert_eq!(content_at(&run.chunks[0], 0), "a".repeat(10));
    assert_eq!(
        content_at(&run.chunks[1], 0),
        format!("{}{}", "a".repeat(50), "b".repeat(10))
    );
    assert_eq!(content_at(&run.chunks[2], 0), "b".repeat(50));
    assert!(run.outcome.is_ok());
}

#[tokio::test]
async fn hook_keeps_a_real_trailing_space_on_a_non_final_chunk() {
    let content = format!("{} ", "a".repeat(60));
    let case = stream_case(vec![chunk(vec![choice(
        Some(0),
        Some(content.as_str()),
        None,
    )])]);
    let run = run_stream(&case).await;
    assert_eq!(content_at(&run.chunks[0], 0), "a".repeat(11));
}

#[tokio::test]
async fn hook_leaves_an_empty_non_final_delta_without_content() {
    let first = "a".repeat(60);
    let case = stream_case(vec![
        chunk(vec![choice(Some(0), Some(first.as_str()), None)]),
        chunk(vec![choice(Some(0), None, None)]),
    ]);
    let run = run_stream(&case).await;
    assert_eq!(content_at(&run.chunks[0], 0), "a".repeat(10));
    assert!(field(choice_of(&run.chunks[1], 0), &["delta", "content"]).is_none());
}

#[tokio::test]
async fn hook_keys_an_unsigned_integer_index() {
    let first = "a".repeat(60);
    let second = "b".repeat(60);
    let case = stream_case(vec![
        chunk(vec![choice_with_index(
            Some(GuardrailValue::UnsignedInteger(5)),
            Some(first.as_str()),
            None,
        )]),
        chunk(vec![choice(Some(0), Some(second.as_str()), Some("stop"))]),
    ]);
    let run = run_stream(&case).await;
    assert_eq!(content_at(&run.chunks[0], 0), "a".repeat(10));
    assert_eq!(content_at(&run.chunks[1], 0), second);
}

#[tokio::test]
async fn hook_keeps_a_holdback_per_choice_index() {
    let first = "a".repeat(60);
    let second = "b".repeat(60);
    let third = "c".repeat(60);
    let case = stream_case(vec![
        chunk(vec![
            choice(Some(0), Some(first.as_str()), None),
            choice(Some(1), Some(second.as_str()), None),
        ]),
        chunk(vec![choice(Some(1), Some(third.as_str()), None)]),
    ]);
    let run = run_stream(&case).await;
    assert_eq!(run.chunks.len(), 2);
    assert_eq!(content_at(&run.chunks[0], 0), "a".repeat(10));
    assert_eq!(content_at(&run.chunks[0], 1), "b".repeat(10));
    assert_eq!(
        content_at(&run.chunks[1], 0),
        format!("{}{}", "b".repeat(50), "c".repeat(10))
    );
}

#[tokio::test]
async fn hook_keys_a_missing_or_non_integer_index_as_choice_zero() {
    let first = "a".repeat(60);
    let second = "b".repeat(60);
    let case = stream_case(vec![
        chunk(vec![choice(None, Some(first.as_str()), None)]),
        chunk(vec![choice_with_index(
            Some(GuardrailValue::Text("1".to_owned())),
            Some(second.as_str()),
            None,
        )]),
        chunk(vec![choice(Some(0), None, Some("stop"))]),
    ]);
    let run = run_stream(&case).await;
    assert_eq!(
        content_at(&run.chunks[1], 0),
        format!("{}{}", "a".repeat(50), "b".repeat(10))
    );
    assert_eq!(content_at(&run.chunks[2], 0), "b".repeat(50));
}

fn word_case(text: &str, blocked_word: BlockedWord) -> StreamCase {
    let mut case = stream_case(vec![chunk(vec![choice(Some(0), Some(text), Some("stop"))])]);
    case.blocked_words = vec![blocked_word];
    case
}

#[tokio::test]
async fn hook_masks_a_blocked_word_in_the_chunk_it_arrives() {
    let case = word_case("hello secret world", masked_word("secret"));
    let run = run_stream(&case).await;
    assert_eq!(
        content_at(&run.chunks[0], 0),
        "hello [KEYWORD_REDACTED] world"
    );
    assert_eq!(text_field(&run.chunks[0], &["id"]), Some("chatcmpl-stream"));
    let record = record(&run);
    assert_eq!(
        text_field(record, &["guardrail_name"]),
        Some("stream-filter")
    );
    assert_eq!(
        text_field(record, &["guardrail_provider"]),
        Some("litellm_content_filter")
    );
    assert_eq!(text_field(record, &["guardrail_mode"]), Some("post_call"));
    assert_eq!(text_field(record, &["guardrail_id"]), Some("stream-filter"));
    assert_eq!(text_field(record, &["guardrail_status"]), Some("success"));
    assert_eq!(text_field(record, &["detection_method"]), Some("keyword"));
    assert_eq!(int_field(record, &["patterns_checked"]), Some(1));
    assert_eq!(
        int_field(record, &["masked_entity_count", "blocked_word"]),
        Some(1)
    );
    assert_eq!(
        field(record, &["risk_score"]),
        Some(&GuardrailValue::Decimal(7.6))
    );
    let detections = response_detections(record);
    assert_eq!(detections.len(), 1);
    assert_eq!(text_field(&detections[0], &["type"]), Some("blocked_word"));
    assert_eq!(text_field(&detections[0], &["keyword"]), Some("secret"));
    assert_eq!(text_field(&detections[0], &["action"]), Some("MASK"));
}

#[tokio::test]
async fn hook_times_a_masked_blocked_word_after_the_epoch() {
    let case = word_case("hello secret world", masked_word("secret"));
    let run = run_stream(&case).await;
    let record = record(&run);
    let start_time = float_field(record, &["start_time"]).expect("the record has a start time");
    let end_time = float_field(record, &["end_time"]).expect("the record has an end time");
    assert!(start_time > 1_700_000_000.0);
    assert!(end_time >= start_time);
    let seconds = float_field(record, &["duration"]).expect("the record has a duration");
    assert!(seconds > 0.0);
}

#[test]
fn hook_logs_the_accumulated_scan_time_as_the_duration() {
    let categories = CategoryLoadResult::default();
    let source = ContentFilterSource {
        categories: &categories,
        patterns: &[],
        blocked_words: &[],
        pattern_redaction_format: "[{pattern_name}_REDACTED]",
        keyword_redaction_tag: "[KEYWORD_REDACTED]",
    };
    let scan = StreamScan {
        source: &source,
        plan: streamed_scan_plan(&source),
        states: IndexMap::new(),
        scan_time: Duration::from_millis(250),
    };
    let mut request_data = BTreeMap::new();
    let mut call = StreamFilterCall {
        user_api_key_dict: None,
        request_data: &mut request_data,
        log_target: None,
    };
    log_stream_record(
        &stream_settings(),
        &scan,
        &mut call,
        GuardrailStatus::Success,
        SystemTime::now(),
    )
    .expect("the record is written");
    let run = StreamRun {
        chunks: Vec::new(),
        request_data,
        outcome: Ok(()),
    };
    assert_eq!(float_field(record(&run), &["duration"]), Some(0.25));
}

#[tokio::test]
async fn hook_logs_the_configured_policy_template() {
    let mut case = stream_case(vec![chunk(vec![choice(
        Some(0),
        Some("hello"),
        Some("stop"),
    )])]);
    case.settings.policy_template = Some("stream policy".to_owned());
    let run = run_stream(&case).await;
    let record = record(&run);
    assert_eq!(
        text_field(record, &["policy_template"]),
        Some("stream policy")
    );
}

#[tokio::test]
async fn hook_yields_a_final_chunk_whole_and_drops_the_synthetic_space() {
    let mut case = stream_case(vec![chunk(vec![choice(
        Some(0),
        Some("hello secret"),
        Some("stop"),
    )])]);
    case.patterns = vec![compiled_pattern("secret(?=\\s)", "peek")];
    let run = run_stream(&case).await;
    assert_eq!(content_at(&run.chunks[0], 0), "hello [PEEK_REDACTED]");
    let record = record(&run);
    assert_eq!(text_field(record, &["detection_method"]), Some("regex"));
    assert_eq!(int_field(record, &["patterns_checked"]), Some(1));
    assert_eq!(int_field(record, &["masked_entity_count", "peek"]), Some(1));
    let detections = response_detections(record);
    assert_eq!(detections.len(), 1);
    assert_eq!(text_field(&detections[0], &["pattern_name"]), Some("peek"));
}

#[tokio::test]
async fn hook_trims_a_non_final_buffer_and_merges_its_committed_detections() {
    let content = format!(
        "{}{}{}{}",
        "alpha",
        "x".repeat(573),
        "alpha",
        "x".repeat(507)
    );
    let mut case = stream_case(vec![
        chunk(vec![choice(Some(0), Some(content.as_str()), None)]),
        chunk(vec![choice(Some(0), None, Some("stop"))]),
    ]);
    case.blocked_words = vec![masked_word("alpha")];
    let run = run_stream(&case).await;
    assert_eq!(content_at(&run.chunks[0], 0).len(), 1066);
    assert_eq!(content_at(&run.chunks[1], 0).len(), 50);
    let record = record(&run);
    let detections = response_detections(record);
    assert_eq!(detections.len(), 1);
    assert_eq!(text_field(&detections[0], &["keyword"]), Some("alpha"));
    assert_eq!(
        int_field(record, &["masked_entity_count", "blocked_word"]),
        Some(1)
    );
    assert_eq!(int_field(record, &["patterns_checked"]), Some(1));
}

#[test]
fn store_choice_trims_only_non_final_chunks() {
    let categories = CategoryLoadResult::default();
    let source = ContentFilterSource {
        categories: &categories,
        patterns: &[],
        blocked_words: &[],
        pattern_redaction_format: "[{pattern_name}_REDACTED]",
        keyword_redaction_tag: "[KEYWORD_REDACTED]",
    };
    let mut scan = StreamScan {
        source: &source,
        plan: streamed_scan_plan(&source),
        states: IndexMap::new(),
        scan_time: Duration::ZERO,
    };
    let buffer = "x".repeat(1100);
    store_choice(&mut scan, 0, true, pending(&buffer, 1100));
    store_choice(&mut scan, 1, false, pending(&buffer, 1100));
    assert_eq!(scan.states[&0].buffered_text.len(), 1100);
    assert_eq!(scan.states[&1].buffered_text.len(), 512);
    assert_eq!(scan.states[&1].yielded_masked_text_len, 512);
}

#[tokio::test]
async fn hook_raises_a_blocked_error_and_logs_the_intervention() {
    let case = word_case(
        "hello blockedword",
        word("blockedword", ContentFilterAction::Block),
    );
    let run = run_stream(&case).await;
    assert!(run.chunks.is_empty());
    let Err(StreamFilterError::Blocked { detail }) = &run.outcome else {
        panic!("the hook reported a different outcome");
    };
    assert_eq!(
        detail,
        &ContentFilterBlockDetail::BlockedWord {
            error: "Content blocked: keyword 'blockedword' detected".to_owned(),
            keyword: "blockedword".to_owned(),
            description: None,
        }
    );
    let error = run
        .outcome
        .as_ref()
        .expect_err("the hook reports the block");
    assert_eq!(error.status_code(), Some(400));
}

#[tokio::test]
async fn hook_logs_the_intervention_when_a_blocked_word_stops_the_stream() {
    let case = word_case(
        "hello blockedword",
        word("blockedword", ContentFilterAction::Block),
    );
    let run = run_stream(&case).await;
    let record = record(&run);
    assert_eq!(
        text_field(record, &["guardrail_status"]),
        Some("guardrail_intervened")
    );
    assert_eq!(
        field(record, &["risk_score"]),
        Some(&GuardrailValue::Decimal(10.0))
    );
    assert_eq!(
        field(record, &["masked_entity_count"]),
        Some(&GuardrailValue::Object(BTreeMap::new()))
    );
    let detections = response_detections(record);
    assert_eq!(detections.len(), 1);
    assert_eq!(
        text_field(&detections[0], &["keyword"]),
        Some("blockedword")
    );
    assert_eq!(text_field(&detections[0], &["action"]), Some("BLOCK"));
}

#[tokio::test]
async fn hook_falls_back_to_unfiltered_text_when_the_scan_fails() {
    let mut case = stream_case(vec![
        chunk(vec![choice(Some(0), Some("alpha "), None)]),
        chunk(vec![choice(Some(0), Some("secret"), Some("stop"))]),
    ]);
    case.patterns = vec![compiled_pattern("secret", "peek")];
    case.settings.pattern_redaction_format = "[{missing}_REDACTED]".to_owned();
    case.blocked_words = vec![masked_word("alpha")];
    let run = run_stream(&case).await;
    // the synthetic space is stripped only when the scan succeeds, so the fallback keeps it
    assert_eq!(content_at(&run.chunks[1], 0), "alpha secret ");
    let detections = response_detections(record(&run));
    assert_eq!(detections.len(), 1);
    assert_eq!(text_field(&detections[0], &["type"]), Some("blocked_word"));
    assert_eq!(text_field(&detections[0], &["keyword"]), Some("alpha"));
}

#[tokio::test]
async fn hook_logs_a_record_when_the_consumer_stops_reading() {
    let first = "a".repeat(60);
    let mut case = stream_case(vec![
        chunk(vec![choice(Some(0), Some(first.as_str()), None)]),
        chunk(vec![choice(Some(0), Some("blockedword"), Some("stop"))]),
    ]);
    case.blocked_words = vec![word("blockedword", ContentFilterAction::Block)];
    case.close_after = Some(1);
    let run = run_stream(&case).await;
    assert_eq!(run.chunks.len(), 1);
    assert_eq!(content_at(&run.chunks[0], 0), "a".repeat(10));
    assert!(run.outcome.is_ok());
    assert_eq!(
        text_field(record(&run), &["guardrail_status"]),
        Some("success")
    );
}

#[tokio::test]
async fn hook_keeps_detections_that_left_the_buffer_and_ones_from_another_choice() {
    let first = format!("alpha{}", "x".repeat(1100));
    let second = format!("beta{}", "y".repeat(1100));
    let mut case = stream_case(vec![
        chunk(vec![
            choice(Some(0), Some(first.as_str()), None),
            choice(Some(1), Some(second.as_str()), None),
        ]),
        chunk(vec![
            choice(Some(0), None, Some("stop")),
            choice(Some(1), None, Some("stop")),
        ]),
    ]);
    case.blocked_words = vec![masked_word("alpha"), masked_word("beta")];
    let run = run_stream(&case).await;
    let record = record(&run);
    let detections = response_detections(record);
    assert_eq!(detections.len(), 2);
    assert_eq!(text_field(&detections[0], &["keyword"]), Some("alpha"));
    assert_eq!(text_field(&detections[1], &["keyword"]), Some("beta"));
    assert_eq!(
        int_field(record, &["masked_entity_count", "blocked_word"]),
        Some(2)
    );
}

#[tokio::test]
async fn hook_logs_one_detection_per_choice_for_the_same_masked_word() {
    let content = format!("secret{}", "x".repeat(60));
    let mut case = stream_case(vec![chunk(vec![
        choice(Some(0), Some(content.as_str()), Some("stop")),
        choice(Some(1), Some(content.as_str()), Some("stop")),
    ])]);
    case.blocked_words = vec![masked_word("secret")];
    let run = run_stream(&case).await;
    let record = record(&run);
    let detections = response_detections(record);
    assert_eq!(detections.len(), 2);
    assert_eq!(text_field(&detections[0], &["keyword"]), Some("secret"));
    assert_eq!(text_field(&detections[1], &["keyword"]), Some("secret"));
    assert_eq!(
        int_field(record, &["masked_entity_count", "blocked_word"]),
        Some(2)
    );
}

#[tokio::test]
async fn hook_keeps_the_yielded_count_when_a_mask_collapses_the_buffer() {
    let first = "1".repeat(60);
    let second = "2".repeat(60);
    let letters = "a".repeat(20);
    let tail = "b".repeat(50);
    let mut case = stream_case(vec![
        chunk(vec![choice(Some(0), Some(first.as_str()), None)]),
        chunk(vec![choice(Some(0), Some(second.as_str()), None)]),
        chunk(vec![choice(Some(0), Some(letters.as_str()), None)]),
        chunk(vec![choice(Some(0), Some(tail.as_str()), Some("stop"))]),
    ]);
    case.patterns = vec![compiled_pattern("[0-9]{100,}", "run")];
    let run = run_stream(&case).await;
    assert_eq!(content_at(&run.chunks[0], 0), "1".repeat(10));
    assert!(content_at(&run.chunks[1], 0).is_empty());
    assert!(content_at(&run.chunks[2], 0).is_empty());
    assert_eq!(
        content_at(&run.chunks[3], 0),
        format!("TED]{}{}", "a".repeat(20), "b".repeat(50))
    );
}

#[tokio::test]
async fn hook_slices_multibyte_text_by_characters() {
    let content = "é".repeat(1100);
    let case = stream_case(vec![
        chunk(vec![choice(Some(0), Some(content.as_str()), None)]),
        chunk(vec![choice(Some(0), None, Some("stop"))]),
    ]);
    let run = run_stream(&case).await;
    assert_eq!(content_at(&run.chunks[0], 0), "é".repeat(1050));
    assert_eq!(content_at(&run.chunks[1], 0), "é".repeat(50));
}

#[tokio::test]
async fn hook_passes_through_items_without_choices() {
    let case = stream_case(vec![
        GuardrailValue::Text("keepalive".to_owned()),
        chunk(Vec::new()),
    ]);
    let run = run_stream(&case).await;
    assert_eq!(run.chunks.len(), 2);
    assert_eq!(run.chunks[0], GuardrailValue::Text("keepalive".to_owned()));
    assert!(matches!(
        field(&run.chunks[1], &["choices"]),
        Some(GuardrailValue::Array(choices)) if choices.is_empty()
    ));
    assert!(run.outcome.is_ok());
}

#[tokio::test]
async fn hook_skips_choices_without_content_and_without_a_final_flag() {
    let second = "b".repeat(60);
    let case = stream_case(vec![chunk(vec![
        choice(Some(0), None, None),
        choice(Some(0), Some(second.as_str()), None),
    ])]);
    let run = run_stream(&case).await;
    assert!(field(choice_of(&run.chunks[0], 0), &["delta", "content"]).is_none());
    assert_eq!(content_at(&run.chunks[0], 1), "b".repeat(10));
}

#[tokio::test]
async fn hook_skips_a_final_chunk_that_buffers_nothing() {
    let case = stream_case(vec![chunk(vec![choice(Some(0), None, Some("stop"))])]);
    let run = run_stream(&case).await;
    assert!(field(choice_of(&run.chunks[0], 0), &["delta", "content"]).is_none());
    assert!(response_detections(record(&run)).is_empty());
}

#[tokio::test]
async fn hook_skips_a_choice_without_a_delta() {
    let without_delta = GuardrailValue::Object(BTreeMap::from([
        ("index".to_owned(), GuardrailValue::SignedInteger(0)),
        ("finish_reason".to_owned(), GuardrailValue::Null),
    ]));
    let with_null_delta = GuardrailValue::Object(BTreeMap::from([
        ("index".to_owned(), GuardrailValue::SignedInteger(0)),
        ("delta".to_owned(), GuardrailValue::Null),
        ("finish_reason".to_owned(), GuardrailValue::Null),
    ]));
    let case = stream_case(vec![chunk(vec![without_delta, with_null_delta])]);
    let run = run_stream(&case).await;
    assert_eq!(run.chunks.len(), 1);
    assert!(field(choice_of(&run.chunks[0], 0), &["delta", "content"]).is_none());
    assert!(field(choice_of(&run.chunks[0], 1), &["delta", "content"]).is_none());
    assert!(run.outcome.is_ok());
}

#[tokio::test]
async fn hook_accepts_the_caller_key_without_reading_it() {
    let mut case = stream_case(vec![chunk(vec![choice(
        Some(0),
        Some("hello"),
        Some("stop"),
    )])]);
    case.user_api_key_dict = Some(GuardrailValue::Object(BTreeMap::from([(
        "api_key".to_owned(),
        GuardrailValue::Text("sk-test".to_owned()),
    )])));
    let run = run_stream(&case).await;
    assert_eq!(content_at(&run.chunks[0], 0), "hello");
    assert!(run.outcome.is_ok());
}

#[test]
fn stream_status_maps_the_outcome_to_a_record_status() {
    assert_eq!(
        stream_status(&Ok::<(), StreamFilterError>(())),
        GuardrailStatus::Success
    );
    let blocked = Err(StreamFilterError::Blocked {
        detail: ContentFilterBlockDetail::BlockedWord {
            error: "Content blocked: keyword 'word' detected".to_owned(),
            keyword: "word".to_owned(),
            description: None,
        },
    });
    assert_eq!(
        stream_status(&blocked),
        GuardrailStatus::GuardrailIntervened
    );
    let failed = Err(StreamFilterError::Record(
        GuardrailRecordError::MessagesShape {
            call_type: "completion".to_owned(),
        },
    ));
    assert_eq!(
        stream_status(&failed),
        GuardrailStatus::GuardrailFailedToRespond
    );
}

#[test]
fn blocked_errors_carry_status_400_and_other_errors_carry_none() {
    let blocked = StreamFilterError::Blocked {
        detail: ContentFilterBlockDetail::BlockedWord {
            error: "Content blocked: keyword 'word' detected".to_owned(),
            keyword: "word".to_owned(),
            description: None,
        },
    };
    assert_eq!(blocked.status_code(), Some(400));
    let failed = StreamFilterError::Record(GuardrailRecordError::MessagesShape {
        call_type: "completion".to_owned(),
    });
    assert_eq!(failed.status_code(), None);
}
