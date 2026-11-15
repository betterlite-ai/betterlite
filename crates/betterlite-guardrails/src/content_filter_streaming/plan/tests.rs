#![cfg(test)]

use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_category::model::CategoryConfig;
use betterlite_config::sections::content_filter_records::category_types::ConditionalCategoryConfig;
use betterlite_config::sections::content_filter_records::{BlockedWord, ContentFilterAction};

use crate::content_filter_actions::ContentFilterSource;
use crate::content_filter_streaming::plan::streamed_scan_plan;

fn category(name: &str, exceptions: &[&str]) -> CategoryConfig {
    CategoryConfig {
        category_name: name.to_owned(),
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

fn conditional(identifier: &[&str], block: &[&str]) -> ConditionalCategoryConfig {
    ConditionalCategoryConfig {
        identifier_words: identifier.iter().map(|word| (*word).to_owned()).collect(),
        block_words: block.iter().map(|word| (*word).to_owned()).collect(),
        action: ContentFilterAction::Mask,
        severity: "medium".to_owned(),
    }
}

fn word(keyword: &str) -> BlockedWord {
    BlockedWord {
        keyword: keyword.to_owned(),
        action: ContentFilterAction::Mask,
        description: None,
    }
}

const fn source<'source>(
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

#[test]
fn context_defaults_to_512_when_no_keyword_is_longer() {
    let categories = CategoryLoadResult::default();
    let words = [word("secret")];
    let source = source(&categories, &words);
    assert_eq!(streamed_scan_plan(&source).context_chars, 512);
}

#[test]
fn context_keeps_512_for_a_keyword_of_exactly_512_characters() {
    let categories = CategoryLoadResult::default();
    let words = [word(&"k".repeat(512))];
    let source = source(&categories, &words);
    assert_eq!(streamed_scan_plan(&source).context_chars, 512);
}

#[test]
fn context_widens_to_the_longest_blocked_word() {
    let categories = CategoryLoadResult::default();
    let words = [word(&"k".repeat(513))];
    let source = source(&categories, &words);
    assert_eq!(streamed_scan_plan(&source).context_chars, 513);
}

#[test]
fn context_widens_to_the_longest_category_keyword() {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.insert(
        "c".repeat(600),
        (
            "stream".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Mask,
        ),
    );
    let source = source(&categories, &[]);
    assert_eq!(streamed_scan_plan(&source).context_chars, 600);
}

#[test]
fn context_widens_to_the_longest_always_block_category_keyword() {
    let mut categories = CategoryLoadResult::default();
    categories.always_block_category_keywords.insert(
        "a".repeat(700),
        (
            "stream".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Block,
        ),
    );
    let source = source(&categories, &[]);
    assert_eq!(streamed_scan_plan(&source).context_chars, 700);
}

#[test]
fn context_takes_the_longest_across_every_keyword_source() {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.insert(
        "c".repeat(600),
        (
            "stream".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Mask,
        ),
    );
    categories.always_block_category_keywords.insert(
        "a".repeat(650),
        (
            "stream".to_owned(),
            "high".to_owned(),
            ContentFilterAction::Block,
        ),
    );
    let words = [word(&"b".repeat(700))];
    let source = source(&categories, &words);
    assert_eq!(streamed_scan_plan(&source).context_chars, 700);
}

#[test]
fn exception_phrases_come_from_every_loaded_category() {
    let mut categories = CategoryLoadResult::default();
    categories
        .loaded_categories
        .insert("first".to_owned(), category("first", &["allowed one"]));
    categories
        .loaded_categories
        .insert("second".to_owned(), category("second", &["allowed two"]));
    let source = source(&categories, &[]);
    assert_eq!(
        streamed_scan_plan(&source).exception_phrases,
        ["allowed one", "allowed two"]
    );
}

#[test]
fn conditional_words_pair_identifier_and_block_words() {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "stream".to_owned(),
        conditional(&["child"], &["harm", "danger"]),
    );
    let source = source(&categories, &[]);
    assert_eq!(
        streamed_scan_plan(&source).conditional_words,
        ["child", "harm", "danger"]
    );
}

#[test]
fn plan_without_categories_has_no_phrases_or_words() {
    let categories = CategoryLoadResult::default();
    let source = source(&categories, &[]);
    let plan = streamed_scan_plan(&source);
    assert_eq!(plan.context_chars, 512);
    assert!(plan.exception_phrases.is_empty());
    assert!(plan.conditional_words.is_empty());
}
