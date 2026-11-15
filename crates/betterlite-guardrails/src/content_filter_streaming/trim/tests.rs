#![cfg(test)]

use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_category::model::{CategoryConfig, CompiledPhrasePattern};
use betterlite_config::sections::content_category::pattern::Pattern;
use betterlite_config::sections::content_filter_records::category_types::ConditionalCategoryConfig;
use betterlite_config::sections::content_filter_records::{
    BlockedWord, BlockedWordDetection, ContentFilterAction, ContentFilterDetection,
    ContentFilterPattern,
};

use crate::content_filter_actions::ContentFilterSource;
use crate::content_filter_matching::CategoryMatch;
use crate::content_filter_streaming::plan::StreamedScanPlan;
use crate::content_filter_streaming::state::{
    CarriedMatches, DetectionCheck, StreamedChoiceState, StreamedDetection,
};
use crate::content_filter_streaming::trim::trim_streamed_choice_buffer;
use crate::pattern_matching::{CompiledPattern, compile};

fn word(keyword: &str, action: ContentFilterAction) -> BlockedWord {
    BlockedWord {
        keyword: keyword.to_owned(),
        action,
        description: None,
    }
}

const fn streamed(detection: ContentFilterDetection) -> StreamedDetection {
    StreamedDetection {
        check: DetectionCheck::BlockedWord,
        position: (0, 0),
        detection,
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

fn conditional(identifiers: &[&str]) -> ConditionalCategoryConfig {
    ConditionalCategoryConfig {
        identifier_words: identifiers
            .iter()
            .map(|identifier| (*identifier).to_owned())
            .collect(),
        block_words: vec!["harm".to_owned()],
        action: ContentFilterAction::Mask,
        severity: "medium".to_owned(),
    }
}

fn phrase_category() -> CategoryLoadResult {
    let mut categories = CategoryLoadResult::default();
    categories
        .loaded_categories
        .insert("stream".to_owned(), category(&["allowed phrase"]));
    categories.category_keywords.insert(
        "target".to_owned(),
        (
            "stream".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Mask,
        ),
    );
    categories
}

const fn source<'source>(
    categories: &'source CategoryLoadResult,
    patterns: &'source [CompiledPattern],
    blocked_words: &'source [BlockedWord],
) -> ContentFilterSource<'source> {
    ContentFilterSource {
        categories,
        patterns,
        blocked_words,
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

fn conditional_plan() -> StreamedScanPlan {
    StreamedScanPlan {
        context_chars: 512,
        exception_phrases: Vec::new(),
        conditional_words: vec!["child".to_owned(), "harm".to_owned()],
    }
}

#[test]
fn trim_returns_the_state_unchanged_while_the_buffer_fits_twice_the_context() {
    let categories = CategoryLoadResult::default();
    let source = source(&categories, &[], &[]);
    let buffered = "x".repeat(1024);
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 974,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &plan());
    assert_eq!(trimmed.buffered_text, buffered);
    assert_eq!(trimmed.yielded_masked_text_len, 974);
    assert_eq!(trimmed.next_trim_len, 0);
}

#[test]
fn trim_cuts_once_the_buffer_passes_twice_the_context() {
    let categories = CategoryLoadResult::default();
    let source = source(&categories, &[], &[]);
    let buffered = "x".repeat(1025);
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 975,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &plan());
    assert_eq!(trimmed.buffered_text, "x".repeat(512));
    assert_eq!(trimmed.yielded_masked_text_len, 462);
    assert_eq!(trimmed.next_trim_len, 0);
}

#[test]
fn trim_trims_when_the_masked_prefix_is_exactly_the_yielded_length() {
    let categories = CategoryLoadResult::default();
    let source = source(&categories, &[], &[]);
    let buffered = "x".repeat(1025);
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 513,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &plan());
    assert_eq!(trimmed.buffered_text, "x".repeat(512));
    assert_eq!(trimmed.yielded_masked_text_len, 0);
    assert_eq!(trimmed.next_trim_len, 0);
}

#[test]
fn trim_defers_while_the_buffer_stays_under_next_trim_len() {
    let categories = CategoryLoadResult::default();
    let source = source(&categories, &[], &[]);
    let buffered = "x".repeat(1100);
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1050,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 2000,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &plan());
    assert_eq!(trimmed.buffered_text.len(), 1100);
    assert_eq!(trimmed.next_trim_len, 2000);
}

#[test]
fn trim_sets_next_trim_len_to_the_buffer_plus_the_context() {
    let categories = CategoryLoadResult::default();
    let words = [word("secret", ContentFilterAction::Mask)];
    let source = source(&categories, &[], &words);
    let buffered = format!("{}secret{}", "x".repeat(585), "x".repeat(509));
    let masked = format!("{}[KEYWORD_REDACTED]{}", "x".repeat(585), "x".repeat(509));
    let state = StreamedChoiceState {
        buffered_text: buffered,
        yielded_masked_text_len: 1050,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 1000,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &masked, &plan());
    assert_eq!(trimmed.next_trim_len, 1100 + 512);
}

#[test]
fn trim_drops_the_prefix_and_remembers_the_exception_phrase() {
    let mut categories = CategoryLoadResult::default();
    categories
        .loaded_categories
        .insert("stream".to_owned(), category(&["allowed phrase"]));
    let source = source(&categories, &[], &[]);
    let buffered = format!("allowed phrase{}", "x".repeat(1085));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1049,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
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
    assert_eq!(
        trimmed.seen_exception_phrases,
        vec!["allowed phrase".to_owned()]
    );
}

#[test]
fn trim_records_a_dropped_phrase_once() {
    let mut categories = CategoryLoadResult::default();
    categories
        .loaded_categories
        .insert("stream".to_owned(), category(&["allowed phrase"]));
    let source = source(&categories, &[], &[]);
    let buffered = format!("allowed phrase{}", "x".repeat(1085));
    let mut open_state = state(&buffered, 1049, CarriedMatches::default());
    open_state.seen_exception_phrases = vec!["allowed phrase".to_owned()];
    let scan_plan = StreamedScanPlan {
        context_chars: 512,
        exception_phrases: vec!["allowed phrase".to_owned()],
        conditional_words: Vec::new(),
    };
    let trimmed = trim_streamed_choice_buffer(&source, open_state, &buffered, &scan_plan);
    assert_eq!(
        trimmed.seen_exception_phrases,
        vec!["allowed phrase".to_owned()]
    );
}

#[test]
fn trim_scans_the_halves_as_the_suppressed_whole_scan_did() {
    let categories = phrase_category();
    let source = source(&categories, &[], &[]);
    let buffered = format!(
        "{} target {}{}allowed phrase{}",
        "x".repeat(10),
        "x".repeat(600),
        "x".repeat(400),
        "x".repeat(6)
    );
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 988,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
    };
    let scan_plan = StreamedScanPlan {
        context_chars: 512,
        exception_phrases: vec!["allowed phrase".to_owned()],
        conditional_words: Vec::new(),
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &scan_plan);
    // the whole buffer holds the phrase, so its scan masks nothing, and the halves must match
    assert_eq!(trimmed.buffered_text.len(), 512);
    assert_eq!(trimmed.yielded_masked_text_len, 988 - 526);
    assert_eq!(trimmed.next_trim_len, 0);
    assert_eq!(
        trimmed.seen_exception_phrases,
        vec!["allowed phrase".to_owned()]
    );
    assert!(trimmed.committed_detections.is_empty());
}

#[test]
fn trim_stays_suppressed_after_the_seen_phrase_left_the_buffer() {
    let categories = phrase_category();
    let source = source(&categories, &[], &[]);
    let buffered = format!(
        "{} target {}{}{}",
        "x".repeat(10),
        "x".repeat(600),
        "x".repeat(414),
        "x".repeat(6)
    );
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 988,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
        seen_exception_phrases: vec!["allowed phrase".to_owned()],
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
    };
    let scan_plan = StreamedScanPlan {
        context_chars: 512,
        exception_phrases: vec!["allowed phrase".to_owned()],
        conditional_words: Vec::new(),
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &scan_plan);
    // the phrase is gone from the buffer but stays in the state, so the keyword in the head stays unmasked
    assert_eq!(trimmed.buffered_text.len(), 512);
    assert_eq!(trimmed.yielded_masked_text_len, 988 - 526);
    assert_eq!(trimmed.next_trim_len, 0);
    assert_eq!(
        trimmed.seen_exception_phrases,
        vec!["allowed phrase".to_owned()]
    );
    assert!(trimmed.committed_detections.is_empty());
}

#[test]
fn trim_trims_when_the_exception_phrase_stays_in_the_tail() {
    let mut categories = CategoryLoadResult::default();
    categories
        .loaded_categories
        .insert("stream".to_owned(), category(&["allowed phrase"]));
    let source = source(&categories, &[], &[]);
    let buffered = format!("{}allowed phrase{}", "x".repeat(600), "x".repeat(486));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1050,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
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
    assert_eq!(
        trimmed.seen_exception_phrases,
        vec!["allowed phrase".to_owned()]
    );
}

#[test]
fn trim_drops_the_prefix_and_remembers_the_open_sentence_text() {
    let mut categories = CategoryLoadResult::default();
    categories
        .conditional_categories
        .insert("stream".to_owned(), conditional(&["child"]));
    let source = source(&categories, &[], &[]);
    let buffered = format!("{}child{}", "x".repeat(580), "y".repeat(515));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1050,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
    };
    let scan_plan = StreamedScanPlan {
        context_chars: 512,
        exception_phrases: Vec::new(),
        conditional_words: vec!["child".to_owned(), "harm".to_owned()],
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &scan_plan);
    assert_eq!(trimmed.buffered_text.len(), 512);
    assert_eq!(trimmed.yielded_masked_text_len, 1050 - 588);
    assert_eq!(trimmed.next_trim_len, 0);
    assert_eq!(
        trimmed.open_sentence,
        format!("{}child{}", "x".repeat(580), "y".repeat(3))
    );
}

#[test]
fn trim_defers_when_a_masked_match_straddles_the_cut() {
    let categories = CategoryLoadResult::default();
    let words = [word("secret", ContentFilterAction::Mask)];
    let source = source(&categories, &[], &words);
    let buffered = format!("{}secret{}", "x".repeat(585), "x".repeat(509));
    let masked = format!("{}[KEYWORD_REDACTED]{}", "x".repeat(585), "x".repeat(509));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1050,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
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
    let source = source(&categories, &patterns, &[]);
    let buffered = format!("{}{}", "x".repeat(588), "1".repeat(512));
    let masked = format!("{}[BIG_REDACTED]", "x".repeat(588));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 552,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &masked, &plan());
    assert_eq!(trimmed.buffered_text, buffered);
    assert_eq!(trimmed.next_trim_len, 1100 + 512);
}

#[test]
fn trim_defers_when_the_head_scan_fails() {
    let categories = CategoryLoadResult::default();
    let words = [word("blockedword", ContentFilterAction::Block)];
    let source = source(&categories, &[], &words);
    let buffered = format!("blockedword{}", "x".repeat(1089));
    let state = StreamedChoiceState {
        buffered_text: buffered.clone(),
        yielded_masked_text_len: 1050,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &buffered, &plan());
    assert_eq!(trimmed.buffered_text, buffered);
    assert_eq!(trimmed.next_trim_len, 1100 + 512);
}

#[test]
fn trim_keeps_the_tail_and_commits_the_head_detections() {
    let categories = CategoryLoadResult::default();
    let words = [word("secret", ContentFilterAction::Mask)];
    let source = source(&categories, &[], &words);
    let buffered = format!("{}secret{}", "x".repeat(10), "x".repeat(1084));
    let masked = format!("{}[KEYWORD_REDACTED]{}", "x".repeat(10), "x".repeat(1084));
    let state = StreamedChoiceState {
        buffered_text: buffered,
        yielded_masked_text_len: 1062,
        committed_detections: Vec::new(),
        latest_detections: vec![streamed(detection("secret"))],
        next_trim_len: 0,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: CarriedMatches::default(),
    };
    let trimmed = trim_streamed_choice_buffer(&source, state, &masked, &plan());
    assert_eq!(trimmed.buffered_text, "x".repeat(512));
    assert_eq!(trimmed.yielded_masked_text_len, 462);
    assert_eq!(trimmed.next_trim_len, 0);
    let committed: Vec<_> = trimmed
        .committed_detections
        .iter()
        .map(|entry| entry.detection.clone())
        .collect();
    assert_eq!(
        serde_json::to_value(&committed).unwrap(),
        serde_json::to_value(vec![detection("secret")]).unwrap()
    );
    assert_eq!(trimmed.latest_detections.len(), 1);
}

fn found(matched: &str, category: &str, action: ContentFilterAction) -> CategoryMatch {
    CategoryMatch {
        matched: matched.to_owned(),
        category: category.to_owned(),
        severity: "high".to_owned(),
        action,
    }
}

fn state(buffered: &str, yielded: usize, carried: CarriedMatches) -> StreamedChoiceState {
    StreamedChoiceState {
        buffered_text: buffered.to_owned(),
        yielded_masked_text_len: yielded,
        committed_detections: Vec::new(),
        latest_detections: Vec::new(),
        next_trim_len: 0,
        seen_exception_phrases: Vec::new(),
        open_sentence: String::new(),
        carried_matches: carried,
    }
}

fn compiled_patterns(sources: &[&str]) -> Vec<CompiledPhrasePattern> {
    sources
        .iter()
        .map(|source| CompiledPhrasePattern {
            source: (*source).to_owned(),
            regex: Pattern::compile(source).unwrap(),
        })
        .collect()
}

#[test]
fn trim_carries_the_dropped_prefixes_first_keyword() {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.insert(
        "alpha".to_owned(),
        (
            "alpha_policy".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Mask,
        ),
    );
    let source = source(&categories, &[], &[]);
    let buffered = format!("alpha {}", "x".repeat(1080));
    let masked = format!("[KEYWORD_REDACTED] {}", "x".repeat(1080));
    let mut open_state = state(&buffered, 1049, CarriedMatches::default());
    open_state.open_sentence = "child ".to_owned();
    let trimmed = trim_streamed_choice_buffer(&source, open_state, &masked, &conditional_plan());
    assert_eq!(trimmed.buffered_text.len(), 512);
    let carried = trimmed
        .carried_matches
        .keyword
        .as_ref()
        .expect("the dropped keyword is carried");
    assert_eq!(carried.matched, "alpha");
    assert_eq!(carried.category, "alpha_policy");
}

#[test]
fn trim_keeps_the_carried_keyword_ahead_of_a_later_one_in_the_head() {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.insert(
        "aaa".to_owned(),
        (
            "alpha_policy".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Mask,
        ),
    );
    categories.category_keywords.insert(
        "zzz".to_owned(),
        (
            "zeta_policy".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Mask,
        ),
    );
    let source = source(&categories, &[], &[]);
    let buffered = format!("zzz {}", "x".repeat(1082));
    let masked = format!("[KEYWORD_REDACTED] {}", "x".repeat(1082));
    let carried = CarriedMatches {
        conditional: None,
        phrase: None,
        keyword: Some(found("aaa", "alpha_policy", ContentFilterAction::Mask)),
    };
    let mut open_state = state(&buffered, 1051, carried);
    open_state.open_sentence = "child ".to_owned();
    let trimmed = trim_streamed_choice_buffer(&source, open_state, &masked, &conditional_plan());
    let winner = trimmed
        .carried_matches
        .keyword
        .as_ref()
        .expect("the keyword stays carried");
    assert_eq!(winner.matched, "aaa");
}

#[test]
fn trim_takes_the_heads_keyword_when_it_comes_first() {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.insert(
        "aaa".to_owned(),
        (
            "alpha_policy".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Mask,
        ),
    );
    categories.category_keywords.insert(
        "zzz".to_owned(),
        (
            "zeta_policy".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Mask,
        ),
    );
    let source = source(&categories, &[], &[]);
    let buffered = format!("aaa {}", "x".repeat(1082));
    let masked = format!("[KEYWORD_REDACTED] {}", "x".repeat(1082));
    let carried = CarriedMatches {
        conditional: None,
        phrase: None,
        keyword: Some(found("zzz", "zeta_policy", ContentFilterAction::Mask)),
    };
    let mut open_state = state(&buffered, 1051, carried);
    open_state.open_sentence = "child ".to_owned();
    let trimmed = trim_streamed_choice_buffer(&source, open_state, &masked, &conditional_plan());
    let winner = trimmed
        .carried_matches
        .keyword
        .as_ref()
        .expect("the head's keyword is carried");
    assert_eq!(winner.matched, "aaa");
    assert_eq!(winner.category, "alpha_policy");
}

#[test]
fn trim_clears_the_carry_when_litellm_commits_the_cut() {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.insert(
        "alpha".to_owned(),
        (
            "alpha_policy".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Mask,
        ),
    );
    let source = source(&categories, &[], &[]);
    let buffered = format!("alpha {}", "x".repeat(1080));
    let masked = format!("[KEYWORD_REDACTED] {}", "x".repeat(1080));
    let carried = CarriedMatches {
        conditional: None,
        phrase: None,
        keyword: Some(found("zeta", "zeta_policy", ContentFilterAction::Mask)),
    };
    let trimmed = trim_streamed_choice_buffer(
        &source,
        state(&buffered, 1049, carried),
        &masked,
        &conditional_plan(),
    );
    assert_eq!(trimmed.buffered_text.len(), 512);
    assert!(trimmed.open_sentence.is_empty());
    assert!(trimmed.carried_matches.keyword.is_none());
}

#[test]
fn trim_keeps_the_carried_conditional_ahead_of_the_same_category_in_the_head() {
    let mut categories = CategoryLoadResult::default();
    categories
        .conditional_categories
        .insert("stream".to_owned(), conditional(&["child", "kid"]));
    let source = source(&categories, &[], &[]);
    let buffered = format!("kid harm {}", "x".repeat(1076));
    let carried = CarriedMatches {
        conditional: Some(found("child + harm", "stream", ContentFilterAction::Mask)),
        phrase: None,
        keyword: None,
    };
    let trimmed = trim_streamed_choice_buffer(
        &source,
        state(&buffered, 1049, carried),
        &buffered,
        &conditional_plan(),
    );
    let winner = trimmed
        .carried_matches
        .conditional
        .as_ref()
        .expect("the conditional stays carried");
    assert_eq!(winner.matched, "child + harm");
}

#[test]
fn trim_takes_the_heads_conditional_when_its_category_comes_first() {
    let mut categories = CategoryLoadResult::default();
    categories
        .conditional_categories
        .insert("alpha_policy".to_owned(), conditional(&["kid"]));
    categories
        .conditional_categories
        .insert("zeta_policy".to_owned(), conditional(&["child"]));
    let source = source(&categories, &[], &[]);
    let buffered = format!("kid harm {}", "x".repeat(1076));
    let carried = CarriedMatches {
        conditional: Some(found(
            "child + harm",
            "zeta_policy",
            ContentFilterAction::Mask,
        )),
        phrase: None,
        keyword: None,
    };
    let trimmed = trim_streamed_choice_buffer(
        &source,
        state(&buffered, 1049, carried),
        &buffered,
        &conditional_plan(),
    );
    let winner = trimmed
        .carried_matches
        .conditional
        .as_ref()
        .expect("the head's conditional is carried");
    assert_eq!(winner.category, "alpha_policy");
}

#[test]
fn trim_takes_the_heads_phrase_pattern_when_it_comes_first() {
    let mut categories = CategoryLoadResult::default();
    let mut config = category(&[]);
    config.phrase_patterns = compiled_patterns(&["first pattern", "second pattern"]);
    categories
        .loaded_categories
        .insert("stream".to_owned(), config);
    let source = source(&categories, &[], &[]);
    let buffered = format!("first pattern {}", "x".repeat(1070));
    let carried = CarriedMatches {
        conditional: None,
        phrase: Some(found(
            "phrase: second pattern",
            "stream",
            ContentFilterAction::Mask,
        )),
        keyword: None,
    };
    let mut open_state = state(&buffered, 1034, carried);
    open_state.open_sentence = "child ".to_owned();
    let trimmed = trim_streamed_choice_buffer(&source, open_state, &buffered, &conditional_plan());
    let winner = trimmed
        .carried_matches
        .phrase
        .as_ref()
        .expect("the head's phrase is carried");
    assert_eq!(winner.matched, "phrase: first pattern");
}

#[test]
fn trim_bounds_the_carried_open_sentence_to_two_contexts() {
    let mut categories = CategoryLoadResult::default();
    categories
        .conditional_categories
        .insert("stream".to_owned(), conditional(&["child"]));
    let source = source(&categories, &[], &[]);
    let buffered = format!("{}{}", "x".repeat(1400), "y".repeat(512));
    let mut open_state = state(&buffered, 1862, CarriedMatches::default());
    open_state.open_sentence = "child ".to_owned();
    let trimmed = trim_streamed_choice_buffer(&source, open_state, &buffered, &conditional_plan());
    assert_eq!(trimmed.buffered_text.len(), 512);
    assert_eq!(trimmed.open_sentence, "x".repeat(1024));
}

#[test]
fn trim_starts_the_open_sentence_after_the_heads_last_terminator() {
    let categories = CategoryLoadResult::default();
    let source = source(&categories, &[], &[]);
    let buffered = format!("aa. bb {} child", "x".repeat(1074));
    let mut open_state = state(&buffered, 1037, CarriedMatches::default());
    open_state.open_sentence = "gone ".to_owned();
    let trimmed = trim_streamed_choice_buffer(&source, open_state, &buffered, &conditional_plan());
    assert_eq!(trimmed.buffered_text.len(), 512);
    assert_eq!(trimmed.open_sentence, format!(" bb {}", "x".repeat(568)));
}

#[test]
fn trim_keeps_the_open_sentence_when_only_the_tail_ends_it() {
    let categories = CategoryLoadResult::default();
    let source = source(&categories, &[], &[]);
    let buffered = format!("child {}.{}", "x".repeat(1069), "y".repeat(500));
    let mut open_state = state(&buffered, 1526, CarriedMatches::default());
    open_state.open_sentence = "child ".to_owned();
    let trimmed = trim_streamed_choice_buffer(&source, open_state, &buffered, &conditional_plan());
    assert_eq!(trimmed.buffered_text.len(), 512);
    assert_eq!(trimmed.open_sentence, "x".repeat(1024));
}
