#![cfg(test)]

use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_category::model::CategoryConfig;
use betterlite_config::sections::content_filter_records::category_types::ConditionalCategoryConfig;
use betterlite_config::sections::content_filter_records::{
    BlockedWord, BlockedWordDetection, ContentFilterAction, ContentFilterDetection,
    ContentFilterPattern,
};

use crate::content_filter_actions::ContentFilterSource;
use crate::content_filter_streaming::plan::StreamedScanPlan;
use crate::content_filter_streaming::state::StreamedChoiceState;
use crate::content_filter_streaming::trim::trim_streamed_choice_buffer;
use crate::pattern_matching::{CompiledPattern, compile};

fn word(keyword: &str, action: ContentFilterAction) -> BlockedWord {
    BlockedWord {
        keyword: keyword.to_owned(),
        action,
        description: None,
    }
}

fn detection(keyword: &str) -> ContentFilterDetection {
    ContentFilterDetection::BlockedWord(BlockedWordDetection {
        r#type: "blocked_word".to_owned(),
        keyword: keyword.to_owned(),
        action: "MASK".to_owned(),
        description: None,
    })
}

fn category(exceptions: &[&str]) -> CategoryConfig {
    CategoryConfig {
        category_name: "stream".to_owned(),
        description: String::new(),
        default_action: ContentFilterAction::Mask,
        keywords: Vec::new(),
        exceptions: exceptions
            .iter()
            .map(|phrase| (*phrase).to_owned())
            .collect(),
        identifier_words: Vec::new(),
        always_block_keywords: Vec::new(),
        inherit_from: None,
        additional_block_words: Vec::new(),
        phrase_patterns: Vec::new(),
    }
}

fn keyword_source<'source>(
    categories: &'source CategoryLoadResult,
    blocked_words: &'source [BlockedWord],
) -> ContentFilterSource<'source> {
    ContentFilterSource {
        categories,
        patterns: &[],
        blocked_words,
        pattern_redaction_format: "[{pattern_name}_REDACTED]",
        keyword_redaction_tag: "[KEYWORD_REDACTED]",
    }
}

fn pattern_source<'source>(
    categories: &'source CategoryLoadResult,
    patterns: &'source [CompiledPattern],
) -> ContentFilterSource<'source> {
    ContentFilterSource {
        categories,
        patterns,
        blocked_words: &[],
        pattern_redaction_format: "[{pattern_name}_REDACTED]",
        keyword_redaction_tag: "[KEYWORD_REDACTED]",
    }
}

fn plan() -> StreamedScanPlan {
    StreamedScanPlan {
        context_chars: 512,
        exception_phrases: Vec::new(),
        conditional_words: Vec::new(),
    }
}

#[test]
fn trim_returns_the_state_unchanged_while_the_buffer_fits_twice_the_context() {
    let categories = CategoryLoadResult::default();
    let source = keyword_source(&categories, &[]);
    let buffered = "x".repeat(1024);
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 974,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &plan());
    assert_eq!(trimmed.buffered_text, buffered);
    assert_eq!(trimmed.yielded_masked_text_len, 974);
    assert_eq!(trimmed.next_trim_len, 0);
}

#[test]
fn trim_cuts_once_the_buffer_passes_twice_the_context() {
    let categories = CategoryLoadResult::default();
    let source = keyword_source(&categories, &[]);
    let buffered = "x".repeat(1025);
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 975,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &plan());
    assert_eq!(trimmed.buffered_text, "x".repeat(512));
    assert_eq!(trimmed.yielded_masked_text_len, 462);
    assert_eq!(trimmed.next_trim_len, 0);
}

#[test]
fn trim_trims_when_the_masked_prefix_is_exactly_the_yielded_length() {
    let categories = CategoryLoadResult::default();
    let source = keyword_source(&categories, &[]);
    let buffered = "x".repeat(1025);
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 513,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &plan());
    assert_eq!(trimmed.buffered_text, "x".repeat(512));
    assert_eq!(trimmed.yielded_masked_text_len, 0);
    assert_eq!(trimmed.next_trim_len, 0);
}

#[test]
fn trim_defers_while_the_buffer_stays_under_next_trim_len() {
    let categories = CategoryLoadResult::default();
    let source = keyword_source(&categories, &[]);
    let buffered = "x".repeat(1100);
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1050,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 2000,
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &plan());
    assert_eq!(trimmed.buffered_text.len(), 1100);
    assert_eq!(trimmed.next_trim_len, 2000);
}

#[test]
fn trim_sets_next_trim_len_to_the_buffer_plus_the_context() {
    let categories = CategoryLoadResult::default();
    let words = [word("secret", ContentFilterAction::Mask)];
    let source = keyword_source(&categories, &words);
    let buffered = format!("{}secret{}", "x".repeat(585), "x".repeat(509));
    let masked = format!("{}[KEYWORD_REDACTED]{}", "x".repeat(585), "x".repeat(509));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1050,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 1000,
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &masked, &plan());
    assert_eq!(trimmed.next_trim_len, 1100 + 512);
}

#[test]
fn trim_defers_when_an_exception_phrase_leaves_the_buffer() {
    let mut categories = CategoryLoadResult::default();
    categories
        .loaded_categories
        .insert("stream".to_owned(), category(&["allowed phrase"]));
    let source = keyword_source(&categories, &[]);
    let buffered = format!("allowed phrase{}", "x".repeat(1085));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1049,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
    };
    let scan_plan = StreamedScanPlan {
        context_chars: 512,
        exception_phrases: vec!["allowed phrase".to_owned()],
        conditional_words: Vec::new(),
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &scan_plan);
    assert_eq!(trimmed.buffered_text, buffered);
    assert_eq!(trimmed.next_trim_len, 1099 + 512);
}

#[test]
fn trim_trims_when_the_exception_phrase_stays_in_the_tail() {
    let mut categories = CategoryLoadResult::default();
    categories
        .loaded_categories
        .insert("stream".to_owned(), category(&["allowed phrase"]));
    let source = keyword_source(&categories, &[]);
    let buffered = format!("{}allowed phrase{}", "x".repeat(600), "x".repeat(486));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1050,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
    };
    let scan_plan = StreamedScanPlan {
        context_chars: 512,
        exception_phrases: vec!["allowed phrase".to_owned()],
        conditional_words: Vec::new(),
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &scan_plan);
    assert_eq!(trimmed.buffered_text.len(), 512);
    assert_eq!(trimmed.yielded_masked_text_len, 462);
    assert_eq!(trimmed.next_trim_len, 0);
}

#[test]
fn trim_defers_when_a_conditional_sentence_crosses_the_cut() {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "stream".to_owned(),
        ConditionalCategoryConfig {
            identifier_words: vec!["child".to_owned()],
            block_words: vec!["harm".to_owned()],
            action: ContentFilterAction::Mask,
            severity: "medium".to_owned(),
        },
    );
    let source = keyword_source(&categories, &[]);
    let buffered = format!("{}child{}", "x".repeat(580), "y".repeat(515));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1050,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
    };
    let scan_plan = StreamedScanPlan {
        context_chars: 512,
        exception_phrases: Vec::new(),
        conditional_words: vec!["child".to_owned(), "harm".to_owned()],
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &scan_plan);
    assert_eq!(trimmed.buffered_text, buffered);
    assert_eq!(trimmed.next_trim_len, 1100 + 512);
}

#[test]
fn trim_defers_when_a_masked_match_straddles_the_cut() {
    let categories = CategoryLoadResult::default();
    let words = [word("secret", ContentFilterAction::Mask)];
    let source = keyword_source(&categories, &words);
    let buffered = format!("{}secret{}", "x".repeat(585), "x".repeat(509));
    let masked = format!("{}[KEYWORD_REDACTED]{}", "x".repeat(585), "x".repeat(509));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1050,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &masked, &plan());
    assert_eq!(trimmed.buffered_text, buffered);
    assert_eq!(trimmed.next_trim_len, 1100 + 512);
}

#[test]
fn trim_defers_when_the_masked_prefix_is_not_yet_yielded() {
    let categories = CategoryLoadResult::default();
    let patterns = [compile(&ContentFilterPattern {
        pattern_type: "regex".to_owned(),
        pattern_name: None,
        pattern: Some("[0-9]{500,}".to_owned()),
        name: Some("big".to_owned()),
        action: ContentFilterAction::Mask,
    })
    .unwrap()];
    let source = pattern_source(&categories, &patterns);
    let buffered = format!("{}{}", "x".repeat(588), "1".repeat(512));
    let masked = format!("{}[BIG_REDACTED]", "x".repeat(588));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 552,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &masked, &plan());
    assert_eq!(trimmed.buffered_text, buffered);
    assert_eq!(trimmed.next_trim_len, 1100 + 512);
}

#[test]
fn trim_defers_when_the_head_scan_fails() {
    let categories = CategoryLoadResult::default();
    let words = [word("blockedword", ContentFilterAction::Block)];
    let source = keyword_source(&categories, &words);
    let buffered = format!("blockedword{}", "x".repeat(1089));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1050,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &plan());
    assert_eq!(trimmed.buffered_text, buffered);
    assert_eq!(trimmed.next_trim_len, 1100 + 512);
}

#[test]
fn trim_keeps_the_tail_and_commits_the_head_detections() {
    let categories = CategoryLoadResult::default();
    let words = [word("secret", ContentFilterAction::Mask)];
    let source = keyword_source(&categories, &words);
    let buffered = format!("{}secret{}", "x".repeat(10), "x".repeat(1084));
    let masked = format!("{}[KEYWORD_REDACTED]{}", "x".repeat(10), "x".repeat(1084));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1062,
        committed_detections: Vec::new(),
        latest_detections: vec![detection("secret")],
        next_trim_len: 0,
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &masked, &plan());
    assert_eq!(trimmed.buffered_text, "x".repeat(512));
    assert_eq!(trimmed.yielded_masked_text_len, 462);
    assert_eq!(trimmed.next_trim_len, 0);
    assert_eq!(
        serde_json::to_value(&trimmed.committed_detections).unwrap(),
        serde_json::to_value(vec![detection("secret")]).unwrap()
    );
    assert_eq!(trimmed.latest_detections.len(), 1);
}
