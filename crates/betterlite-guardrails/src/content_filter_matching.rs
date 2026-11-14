use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_category::pattern::PatternError;
use betterlite_config::sections::content_filter_records::ContentFilterAction;

pub mod conditional;
pub mod keywords;
pub mod phrase;
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
