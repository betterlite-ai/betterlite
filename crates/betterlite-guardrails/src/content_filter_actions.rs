pub mod error;
pub mod handlers;
mod tests;

use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_filter_records::{BlockedWord, ContentFilterDetection};
use regex::Regex;

use crate::content_filter_matching::conditional::check_conditional_categories;
use crate::content_filter_matching::keywords::check_category_keywords;
use crate::content_filter_matching::phrase::check_phrase_patterns;
use crate::pattern_matching::CompiledPattern;
use crate::pattern_matching::masking::keyword_pattern;

use self::error::ContentFilterActionError;
use self::handlers::{
    handle_blocked_word_match, handle_category_keyword_match, handle_conditional_match,
    handle_pattern_match,
};

#[derive(Debug)]
pub struct ContentFilterSource<'a> {
    pub categories: &'a CategoryLoadResult,
    pub patterns: &'a [CompiledPattern],
    pub blocked_words: &'a [BlockedWord],
    pub pattern_redaction_format: &'a str,
    pub keyword_redaction_tag: &'a str,
}

pub fn filter_single_text(
    source: &ContentFilterSource<'_>,
    text: &str,
    mut detections: Option<&mut Vec<ContentFilterDetection>>,
) -> Result<String, ContentFilterActionError> {
    let exceptions = matching_exceptions(source.categories);
    if let Some(found) = check_conditional_categories(text, &exceptions, source.categories)? {
        handle_conditional_match(&found, detections.as_deref_mut())?;
    }
    if let Some(found) = check_phrase_patterns(text, &exceptions, source.categories)? {
        handle_conditional_match(&found, detections.as_deref_mut())?;
    }
    let mut filtered = text.to_owned();
    if let Some(found) = check_category_keywords(text, &exceptions, source.categories)? {
        filtered = handle_category_keyword_match(
            &found,
            &filtered,
            source.keyword_redaction_tag,
            detections.as_deref_mut(),
        )?;
    }
    let filtered = mask_patterns(source, &filtered, detections.as_deref_mut())?;
    mask_blocked_words(source, &filtered, detections)
}

fn matching_exceptions(categories: &CategoryLoadResult) -> Vec<String> {
    categories
        .loaded_categories
        .values()
        .flat_map(|category| category.exceptions.iter().cloned())
        .collect()
}

fn mask_patterns(
    source: &ContentFilterSource<'_>,
    text: &str,
    mut detections: Option<&mut Vec<ContentFilterDetection>>,
) -> Result<String, ContentFilterActionError> {
    let mut filtered = text.to_owned();
    for pattern in source.patterns {
        let spans = pattern.find_spans(&filtered)?;
        if spans.is_empty() {
            continue;
        }
        filtered = handle_pattern_match(
            pattern,
            &filtered,
            &spans,
            source.pattern_redaction_format,
            detections.as_deref_mut(),
        )?;
    }
    Ok(filtered)
}

fn mask_blocked_words(
    source: &ContentFilterSource<'_>,
    text: &str,
    mut detections: Option<&mut Vec<ContentFilterDetection>>,
) -> Result<String, ContentFilterActionError> {
    let mut filtered = text.to_owned();
    let mut text_lower = filtered.to_lowercase();
    for word in source.blocked_words {
        if !Regex::new(&keyword_pattern(&word.keyword))?.is_match(&text_lower) {
            continue;
        }
        filtered = handle_blocked_word_match(
            word,
            &filtered,
            source.keyword_redaction_tag,
            detections.as_deref_mut(),
        )?;
        text_lower = filtered.to_lowercase();
    }
    Ok(filtered)
}
