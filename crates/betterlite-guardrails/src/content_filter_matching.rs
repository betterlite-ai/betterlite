use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_category::model::CategoryConfig;
use betterlite_config::sections::content_category::pattern::PatternError;
use betterlite_config::sections::content_filter_records::ContentFilterAction;
use betterlite_config::sections::content_filter_records::category_types::ConditionalCategoryConfig;
use regex::Regex;

use crate::pattern_matching::masking::keyword_pattern;

mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryMatch {
    pub matched: String,
    pub category: String,
    pub severity: String,
    pub action: ContentFilterAction,
}

#[derive(Debug, thiserror::Error)]
pub enum ContentFilterMatchingError {
    #[error(transparent)]
    PhrasePattern(#[from] PatternError),
    #[error(transparent)]
    Keyword(#[from] regex::Error),
}

pub fn check_conditional_categories(
    text: &str,
    exceptions: &[String],
    categories: &CategoryLoadResult,
) -> Result<Option<CategoryMatch>, ContentFilterMatchingError> {
    let text_lower = text.to_lowercase();
    if exceptions
        .iter()
        .any(|exception| text_lower.contains(exception.as_str()))
    {
        return Ok(None);
    }
    let sentences = text
        .split(['.', '!', '?'])
        .map(|sentence| sentence.to_lowercase().trim().to_owned())
        .collect::<Vec<_>>();
    for (category_name, config) in &categories.conditional_categories {
        if category_exception_present(categories, category_name, &text_lower) {
            continue;
        }
        if let Some(found) = match_conditional_category(category_name, config, &sentences)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn match_conditional_category(
    category_name: &str,
    config: &ConditionalCategoryConfig,
    sentences: &[String],
) -> Result<Option<CategoryMatch>, ContentFilterMatchingError> {
    for sentence in sentences {
        if sentence.is_empty() {
            continue;
        }
        let Some(identifier) = config
            .identifier_words
            .iter()
            .find(|word| sentence.contains(word.as_str()))
        else {
            continue;
        };
        for block_word in &config.block_words {
            let found = if block_word.contains(' ') {
                sentence.contains(block_word.as_str())
            } else {
                Regex::new(&format!(r"\b{}\b", regex::escape(block_word)))?.is_match(sentence)
            };
            if found {
                return Ok(Some(CategoryMatch {
                    matched: format!("{identifier} + {block_word}"),
                    category: category_name.to_owned(),
                    severity: config.severity.clone(),
                    action: config.action,
                }));
            }
        }
    }
    Ok(None)
}

pub fn check_phrase_patterns(
    text: &str,
    exceptions: &[String],
    categories: &CategoryLoadResult,
) -> Result<Option<CategoryMatch>, ContentFilterMatchingError> {
    let text_lower = text.to_lowercase();
    if exceptions
        .iter()
        .any(|exception| text_lower.contains(exception.as_str()))
    {
        return Ok(None);
    }
    for (category_name, config) in &categories.loaded_categories {
        if category_exception_present(categories, category_name, &text_lower) {
            continue;
        }
        let action = categories
            .conditional_categories
            .get(category_name)
            .map_or(config.default_action, |conditional| conditional.action);
        if let Some(found) = first_phrase_match(category_name, config, text, action)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn first_phrase_match(
    category_name: &str,
    config: &CategoryConfig,
    text: &str,
    action: ContentFilterAction,
) -> Result<Option<CategoryMatch>, ContentFilterMatchingError> {
    for pattern in &config.phrase_patterns {
        if pattern.regex.is_match(text)? {
            // LiteLLM returns the source pattern with a phrase prefix and always severity high
            return Ok(Some(CategoryMatch {
                matched: format!("phrase: {}", pattern.source),
                category: category_name.to_owned(),
                severity: "high".to_owned(),
                action,
            }));
        }
    }
    Ok(None)
}

pub fn check_category_keywords(
    text: &str,
    exceptions: &[String],
    categories: &CategoryLoadResult,
) -> Result<Option<CategoryMatch>, ContentFilterMatchingError> {
    let text_lower = text.to_lowercase();
    if exceptions
        .iter()
        .any(|exception| text_lower.contains(exception.as_str()))
    {
        return Ok(None);
    }
    for (keyword, (category, severity, action)) in &categories.always_block_category_keywords {
        if keyword_regex(keyword)?.is_match(&text_lower) {
            return Ok(Some(CategoryMatch {
                matched: keyword.clone(),
                category: category.clone(),
                severity: severity.clone(),
                action: *action,
            }));
        }
    }
    for (keyword, (category, severity, action)) in &categories.category_keywords {
        if !keyword_regex(keyword)?.is_match(&text_lower) {
            continue;
        }
        if category_exception_present(categories, category, &text_lower) {
            continue;
        }
        return Ok(Some(CategoryMatch {
            matched: keyword.clone(),
            category: category.clone(),
            severity: severity.clone(),
            action: *action,
        }));
    }
    Ok(None)
}

fn category_exception_present(
    categories: &CategoryLoadResult,
    category: &str,
    text_lower: &str,
) -> bool {
    let Some(config) = categories.loaded_categories.get(category) else {
        return false;
    };
    config
        .exceptions
        .iter()
        .any(|exception| text_lower.contains(exception.as_str()))
}

fn keyword_regex(keyword: &str) -> Result<Regex, regex::Error> {
    let pattern = keyword_pattern(keyword);
    if keyword.contains(' ') {
        return Regex::new(&pattern);
    }
    Regex::new(&format!(r"\b{pattern}\b"))
}
