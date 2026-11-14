use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_category::pattern::PatternError;
use betterlite_config::sections::content_filter_records::ContentFilterAction;

mod conditional;
mod keywords;
mod phrase;
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
    conditional::check(categories, text, exceptions)
}

pub fn check_phrase_patterns(
    text: &str,
    exceptions: &[String],
    categories: &CategoryLoadResult,
) -> Result<Option<CategoryMatch>, ContentFilterMatchingError> {
    phrase::check(categories, text, exceptions)
}

pub fn check_category_keywords(
    text: &str,
    exceptions: &[String],
    categories: &CategoryLoadResult,
) -> Result<Option<CategoryMatch>, ContentFilterMatchingError> {
    keywords::check(categories, text, exceptions)
}

fn global_exception_present(exceptions: &[String], text_lower: &str) -> bool {
    exceptions
        .iter()
        .any(|exception| text_lower.contains(exception.as_str()))
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
