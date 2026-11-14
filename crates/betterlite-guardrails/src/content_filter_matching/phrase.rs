use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_category::model::CategoryConfig;
use betterlite_config::sections::content_filter_records::ContentFilterAction;

use super::CategoryMatch;
use super::ContentFilterMatchingError;
use super::category_exception_present;
use super::global_exception_present;

pub(super) fn check(
    categories: &CategoryLoadResult,
    text: &str,
    exceptions: &[String],
) -> Result<Option<CategoryMatch>, ContentFilterMatchingError> {
    let text_lower = text.to_lowercase();
    if global_exception_present(exceptions, &text_lower) {
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
        if !pattern.regex.is_match(text)? {
            continue;
        }
        // LiteLLM returns the source pattern with a phrase prefix and always severity high
        return Ok(Some(CategoryMatch {
            matched: format!("phrase: {}", pattern.source),
            category: category_name.to_owned(),
            severity: "high".to_owned(),
            action,
        }));
    }
    Ok(None)
}
