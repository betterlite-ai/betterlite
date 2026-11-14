use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_filter_records::category_types::ConditionalCategoryConfig;
use regex::Regex;

use super::CategoryMatch;
use super::ContentFilterMatchingError;
use super::category_exception_present;
use super::global_exception_present;

pub fn check_conditional_categories(
    text: &str,
    exceptions: &[String],
    categories: &CategoryLoadResult,
) -> Result<Option<CategoryMatch>, ContentFilterMatchingError> {
    let text_lower = text.to_lowercase();
    if global_exception_present(exceptions, &text_lower) {
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
